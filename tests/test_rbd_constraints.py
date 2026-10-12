"""Behavioral tests against actual MuJoCo, not mocked constraint-name mappings."""
import importlib.util
import json
from pathlib import Path
import unittest

import numpy as np

from minihou import Evaluator, Graph, NodeError
from minihou.rbd import World


def scene(kind, *, center=(1, 0, 2), size=(.4, .4, .4), world=True, other_active=False,
          velocity=(0, 0, 0), angular=(0, 0, 0), **params):
    return {
        "box": {"type": "box", "params": {"center": list(center), "size": list(size)}},
        "a": {"type": "rbd_configure", "inputs": {"geo": "box"}, "params": {
            "linear_damping": 0, "angular_damping": 0, "velocity": list(velocity), "angular_velocity": list(angular)}},
        "support": {"type": "box", "params": {"center": [0, 0, 2], "size": [.2, .2, .2]}},
        "b": {"type": "rbd_configure", "inputs": {"geo": "support"}, "params": {
            "id_offset": 1, "active": other_active, "linear_damping": 0, "angular_damping": 0}},
        "bodies": {"type": "merge", "inputs": {"geos": ["a"] if world else ["a", "b"]}},
        "link": {"type": "rbd_constraint", "params": dict(kind=kind, world=world, anchor=[0, 0, 2], damping=0, **params)},
        "sim": {"type": "rbd_solver", "inputs": {"bodies": "bodies", "constraints": "link"}},
    }


class ConstraintSchemaTests(unittest.TestCase):
    def cook(self, params):
        ev = Evaluator()
        self.addCleanup(ev.close)
        return ev.cook(Graph.from_dict({"nodes": {"c": {"type": "rbd_constraint", "params": params}}, "display": "c"}))

    def test_kinds_emit_mergeable_typed_links(self):
        for code, kind in enumerate(("glue", "pin", "hinge", "spring")):
            with self.subTest(kind=kind):
                geo = self.cook(dict(kind=kind))
                self.assertEqual(geo.nprims, 1)
                self.assertEqual(int(geo.prim["rbd_constraint"][0]), code)
                self.assertEqual(int(geo.prim["rbd_schema"][0]), 1)
                self.assertEqual(geo.prim["axis"].shape, (1, 3))

    def test_invalid_parameters_have_actionable_errors(self):
        for params, text in [
            (dict(kind="hinge", axis=[0, 0, 0]), "axis"),
            (dict(kind="hinge", limited=True, lower=20, upper=30), "zero"),
            (dict(kind="hinge", limited=True, lower=0, upper=0), "Upper"),
            (dict(kind="spring", limited=True, lower=-1, upper=2), "negative"),
            (dict(kind="spring", rest_length=-.5), "Rest Length"),
            (dict(kind="spring", anchor_b=[0, 0, 0]), "different positions"),
            (dict(kind="pin", break_torque=1), "no direct torque"),
            (dict(kind="pin", limited=True), "limits apply"),
            (dict(body_a=0, body_b=0), "different body IDs"),
        ]:
            with self.subTest(params=params), self.assertRaisesRegex(NodeError, text):
                self.cook(params)


@unittest.skipUnless(importlib.util.find_spec("mujoco"), "optional physics extra not installed")
class ConstraintPhysicsTests(unittest.TestCase):
    def world(self, nodes, gravity=(0, 0, -9.81), substeps=20):
        ev = Evaluator()
        self.addCleanup(ev.close)
        graph = Graph.from_dict({"nodes": nodes, "display": "bodies"})
        bodies, constraints = ev.cook(graph, target="bodies"), ev.cook(graph, target="link")
        world = World(bodies, None, dict(gravity=gravity, substeps=substeps, iterations=100,
                      margin=.001, start_frame=1, cache_mb=8, include_colliders=False), constraints)
        self.addCleanup(world.close)
        return world

    @staticmethod
    def attachment(world, body, position):
        bid = world.body_ids[body]
        return world.data.xpos[bid] + world.data.xmat[bid].reshape(3, 3) @ (np.array(position)-world.pieces[body].center)

    def test_pin_holds_off_center_anchor_but_allows_rotation(self):
        w = self.world(scene("pin"))
        self.assertEqual(int(w.model.eq_type[0]), int(w.mj.mjtEq.mjEQ_CONNECT))
        start = w.pose(1)[0][0].copy()
        end = w.pose(25)[0][0]
        np.testing.assert_allclose(self.attachment(w, 0, [0, 0, 2]), [0, 0, 2], atol=.004)
        self.assertGreater(np.linalg.norm(end-start), .5)
        self.assertFalse(w.broken[0])

    def test_pin_breaks_under_load_and_replay_restores_constraint(self):
        w = self.world(scene("pin", center=(0, 0, 2), strength=5))
        end = w.pose(25)[0].copy()
        self.assertTrue(w.broken[0])
        self.assertEqual(w.data.eq_active[0], 0)
        self.assertLess(end[0, 2], -1)
        w.restart()
        self.assertFalse(w.broken[0])
        self.assertEqual(w.data.eq_active[0], 1)
        w.frames.clear()
        np.testing.assert_allclose(w.pose(25)[0], end, atol=1e-10)

    def test_pin_allows_spin_while_hinge_rejects_off_axis_spin(self):
        pin = self.world(scene("pin", center=(0, 0, 2), angular=(3, 0, 0)), gravity=(0, 0, 0))
        hinge = self.world(scene("hinge", center=(0, 0, 2), angular=(3, 0, 0)), gravity=(0, 0, 0))
        pin.pose(12)
        hinge.pose(12)
        pin_R = pin.data.xmat[pin.body_ids[0]].reshape(3, 3)
        hinge_R = hinge.data.xmat[hinge.body_ids[0]].reshape(3, 3)
        self.assertGreater(np.linalg.norm(pin_R @ [0, 1, 0]-[0, 1, 0]), .5)
        np.testing.assert_allclose(hinge_R @ [0, 1, 0], [0, 1, 0], atol=.003)

    def test_hinge_rotates_only_about_axis_and_enforces_both_stops(self):
        for axis_sign in (1, -1):
            with self.subTest(sign=axis_sign):
                w = self.world(scene("hinge", axis=[0, axis_sign, 0], limited=True, lower=-25, upper=35))
                jid = w.link_joints[0]
                self.assertEqual(int(w.model.jnt_type[jid]), int(w.mj.mjtJoint.mjJNT_HINGE))
                w.pose(70)
                angle = np.degrees(w.data.qpos[w.model.jnt_qposadr[jid]])
                self.assertAlmostEqual(angle, 35 if axis_sign > 0 else -25, delta=.8)
                np.testing.assert_allclose(self.attachment(w, 0, [0, 0, 2]), [0, 0, 2], atol=.006)
                R = w.data.xmat[w.body_ids[0]].reshape(3, 3)
                np.testing.assert_allclose(R @ [0, 1, 0], [0, 1, 0], atol=.002)

    def test_hinge_between_dynamic_bodies_keeps_shared_pivot(self):
        w = self.world(scene("hinge", world=False, other_active=True, angular=(0, 2, 0)), gravity=(0, 0, 0))
        w.pose(30)
        a = self.attachment(w, 0, [0, 0, 2])
        b = self.attachment(w, 1, [0, 0, 2])
        np.testing.assert_allclose(a, b, atol=.01)
        self.assertGreater(np.linalg.norm(w.data.xmat[w.body_ids[0]].reshape(3, 3)-np.eye(3)), .1)
        self.assertGreater(np.linalg.norm(w.data.xpos[w.body_ids[1]]-[0, 0, 2]), .01)

    def test_hinge_break_torque_releases_body_from_stop(self):
        w = self.world(scene("hinge", limited=True, lower=-1, upper=1, break_torque=2))
        w.pose(35)
        self.assertTrue(w.broken[0])
        self.assertEqual(w.data.eq_active[w.link_equalities[0]], 0)
        self.assertLess(w.data.xpos[w.body_ids[0], 2], -2)

    def test_hinge_breaks_at_nonzero_angle_and_restores_helper_on_restart(self):
        nodes = scene("hinge", limited=True, lower=-25, upper=35, break_torque=5)
        nodes["link"]["params"]["damping"] = .1
        w = self.world(nodes)
        jid = w.link_joints[0]
        for frame in range(2, 50):
            w.pose(frame)
            if w.broken[0]:
                break
        self.assertTrue(w.broken[0])
        angle = np.degrees(w.data.qpos[w.model.jnt_qposadr[jid]])
        self.assertGreater(angle, 25)
        self.assertLess(angle, 40)
        self.assertEqual(w.model.jnt_limited[jid], 0)
        self.assertEqual(w.model.dof_damping[w.model.jnt_dofadr[jid]], 0)
        w.restart()
        self.assertEqual(w.model.jnt_limited[jid], 1)
        self.assertAlmostEqual(w.model.dof_damping[w.model.jnt_dofadr[jid]], .1)

    def test_default_substeps_static_body_attachment_is_stable(self):
        for kind in ("pin", "hinge"):
            with self.subTest(kind=kind):
                w = self.world(scene(kind, world=False), substeps=10)
                w.pose(80)
                np.testing.assert_allclose(self.attachment(w, 0, [0, 0, 2]), [0, 0, 2], atol=.015)
                self.assertFalse(w.broken[0])

    def test_new_links_reject_ambiguous_or_missing_configured_body_ids(self):
        for invalid in ("unknown", "duplicate", "static_hinge"):
            nodes = scene("hinge", world=False)
            if invalid == "unknown":
                nodes["link"]["params"]["body_b"] = 17
            elif invalid == "duplicate":
                nodes["b"]["params"]["id_offset"] = 0
            else:
                nodes["a"]["params"]["active"] = False
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                self.world(nodes)

    def test_spring_settles_at_hooke_equilibrium(self):
        nodes = scene("spring", center=(0, 0, 2), anchor_b=[0, 0, 3], stiffness=100, rest_length=1)
        nodes["link"]["params"]["damping"] = 10
        w = self.world(nodes)
        w.pose(120)
        self.assertEqual(w.model.ntendon, 1)
        self.assertAlmostEqual(w.data.ten_length[0], 1+9.81/100, delta=.002)
        self.assertLess(abs(w.data.ten_velocity[0]), .002)
        self.assertFalse(w.broken[0])

    def test_spring_limits_extension_and_compression(self):
        for velocity, expected in [((0, 0, -3), 1.2), ((0, 0, 3), .8)]:
            with self.subTest(velocity=velocity):
                w = self.world(scene("spring", center=(0, 0, 2), velocity=velocity, anchor_b=[0, 0, 3],
                    stiffness=0, limited=True, lower=.8, upper=1.2), gravity=(0, 0, 0))
                lengths = []
                for frame in range(2, 25):
                    w.pose(frame)
                    lengths.append(w.data.ten_length[0])
                self.assertGreaterEqual(min(lengths), .79)
                self.assertLessEqual(max(lengths), 1.21)
                self.assertLess(min(abs(x-expected) for x in lengths), .01)

    def test_spring_break_disables_force_and_limits_and_restart_restores_them(self):
        w = self.world(scene("spring", center=(0, 0, 2), anchor_b=[0, 0, 3], stiffness=100,
            strength=5, limited=True, lower=.9, upper=1.1))
        w.pose(35)
        self.assertTrue(w.broken[0])
        self.assertEqual(w.model.tendon_stiffness[0], 0)
        self.assertEqual(w.model.tendon_limited[0], 0)
        self.assertLess(w.data.xpos[w.body_ids[0], 2], -2)
        w.restart()
        self.assertEqual(w.model.tendon_stiffness[0], 100)
        self.assertEqual(w.model.tendon_limited[0], 1)

    def test_spring_between_dynamic_bodies_preserves_center_of_mass(self):
        nodes = scene("spring", world=False, other_active=True, center=(1, 0, 2),
                      anchor_b=[0, 0, 2], rest_length=.5, stiffness=20)
        nodes["link"]["params"].update(anchor=[1, 0, 2], damping=2)
        w = self.world(nodes, gravity=(0, 0, 0), substeps=10)
        initial = w.pose(1)[0].mean(0)
        final = w.pose(100)[0]
        np.testing.assert_allclose(final.mean(0), initial, atol=1e-5)
        self.assertAlmostEqual(np.linalg.norm(final[0]-final[1]), .5, delta=.01)

    def test_spring_damping_removes_oscillation_and_force_thresholds_differ(self):
        peaks = []
        for damping in (0, 10):
            nodes = scene("spring", center=(0, 0, 2), anchor_b=[0, 0, 3], rest_length=1,
                          stiffness=100, strength=100)
            nodes["link"]["params"]["damping"] = damping
            w = self.world(nodes)
            speed = []
            for frame in range(50, 90):
                w.pose(frame)
                speed.append(abs(w.data.ten_velocity[0]))
            peaks.append(max(speed))
            self.assertFalse(w.broken[0])
        self.assertGreater(peaks[0], .1)
        self.assertLess(peaks[1], peaks[0]*.02)

    def test_spring_limit_reaction_can_break_without_spring_stiffness(self):
        w = self.world(scene("spring", center=(0, 0, 2), anchor_b=[0, 0, 3], stiffness=0,
                            limited=True, lower=.9, upper=1.1, strength=5))
        w.pose(25)
        self.assertTrue(w.broken[0])
        self.assertGreater(w.data.ten_length[0], 2)

    def test_mixed_links_keep_independent_ids_and_break_only_the_weak_link(self):
        nodes = scene("pin", center=(0, 0, 2), strength=5)
        # Three additional independently configured bodies and link mechanisms.
        for i, kind in enumerate(("hinge", "spring", "glue"), 1):
            nodes[f"box{i}"] = {"type": "box", "params": {"center": [3*i, 0, 2], "size": [.3, .3, .3]}}
            nodes[f"body{i}"] = {"type": "rbd_configure", "inputs": {"geo": f"box{i}"}, "params": {"id_offset": i}}
            nodes[f"link{i}"] = {"type": "rbd_constraint", "params": {"kind": kind, "body_a": i,
                "world": True, "anchor": [3*i, 0, 2], "anchor_b": [3*i, 0, 3]}}
        nodes["bodies"]["inputs"]["geos"] += [f"body{i}" for i in range(1, 4)]
        nodes["pin"] = nodes.pop("link")
        nodes["link"] = {"type": "merge", "inputs": {"geos": ["pin", "link1", "link2", "link3"]}}
        w = self.world(nodes)
        out = w.output(30)
        self.assertEqual(w.broken.tolist(), [True, False, False, False])
        self.assertEqual(out.detail["rbd"]["pin"]["broken_links"], [0])
        self.assertEqual(out.detail["rbd"]["glue"]["broken"], 0)

    def test_connected_collision_exclusion_is_restored_after_break(self):
        w = self.world(scene("pin", world=False, strength=.001))
        signature = w.model.exclude_signature.copy()
        self.assertEqual(len(signature), 1)
        w.pose(30)
        self.assertTrue(w.broken[0])
        self.assertEqual(w.model.exclude_signature[0], -1)
        w.restart()
        np.testing.assert_array_equal(w.model.exclude_signature, signature)

    def test_reversed_link_order_and_hinge_helper_restore_the_correct_pair(self):
        nodes = scene("pin", world=False, strength=.001)
        nodes["cbox"] = {"type": "box", "params": {"center": [3, 0, 2]}}
        nodes["c"] = {"type": "rbd_configure", "inputs": {"geo": "cbox"}, "params": {"id_offset": 2}}
        nodes["bodies"]["inputs"]["geos"].append("c")
        nodes["weak"] = nodes.pop("link")
        nodes["strong"] = {"type": "rbd_constraint", "params": {"kind": "hinge", "body_a": 2,
            "body_b": 1, "anchor": [2, 0, 2]}}
        nodes["link"] = {"type": "merge", "inputs": {"geos": ["strong", "weak"]}}
        w = self.world(nodes)
        strong_slot = w.pairs[(1, 2)][0]
        weak_slot = w.pairs[(0, 1)][0]
        strong_signature = w.model.exclude_signature[strong_slot]
        self.assertNotEqual(strong_slot, weak_slot)
        w.pose(30)
        self.assertEqual(w.broken.tolist(), [False, True])
        self.assertEqual(w.model.exclude_signature[weak_slot], -1)
        self.assertEqual(w.model.exclude_signature[strong_slot], strong_signature)

    def test_examples_show_strong_reference_and_broken_comparison(self):
        root = Path(__file__).resolve().parents[1]
        for kind in ("pin", "hinge", "spring"):
            with self.subTest(kind=kind):
                graph = Graph.from_dict(json.loads((root / "examples" / f"rbd_{kind}_constraints.json").read_text(encoding="utf-8")))
                ev = Evaluator()
                self.addCleanup(ev.close)
                first = ev.cook(graph, frame=1)
                self.assertEqual(first.detail["rbd"][kind]["broken"], 0)
                final = ev.cook(graph, frame=100)
                self.assertEqual(final.detail["rbd"][kind]["broken_links"], [1])
                self.assertTrue(np.isfinite(final.P).all())
                self.assertLess(final.P[8:16, 2].mean(), first.P[8:16, 2].mean()-1)


if __name__ == "__main__":
    unittest.main()
