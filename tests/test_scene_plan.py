"""The bpy-free planner of the add-on's scene objects (what to create, move, retarget, show and hide)."""
import unittest

from tests.addon_loader import pure_module

sp = pure_module("scene_plan")


def manifest(ids=(0, 1, 2), src=None, prefix="Tree", sources=("Tree", "Bush"), loc=None):
    n = len(ids)
    return {"type": "scene_objects", "node": "so", "key": "k", "prefix": prefix, "sources": list(sources), "id": list(ids),
            "src": list(src) if src is not None else [0] * n, "loc": loc or [[float(i), 0.0, 0.0] for i in ids],
            "rot": [[0.0, 0.0, 0.0]] * n, "scale": [[1.0, 1.0, 1.0]] * n}


def have(m, **override):
    """Existing objects exactly as the manifest describes them."""
    out = {}
    for it in sp.items(m):
        out[it.ident] = sp.Existing(it.ident, it.name, it.src, False, False, tuple(it.loc), tuple(it.rot), tuple(it.scale), "")
    for ident, kw in override.items():
        for k, v in kw.items():
            setattr(out[int(ident[1:])], k, v)
    return out


class PlannerTests(unittest.TestCase):
    def test_names_are_prefix_and_zero_padded_id(self):
        self.assertEqual(sp.object_name("Tree", 7), "Tree_0007")
        self.assertEqual(sp.object_name("Tree", 12345), "Tree_12345")
        self.assertEqual(sp.object_name("Tree", -3), "Tree_n0003")
        self.assertEqual(sp.owner_tag("session", "so"), "session:so")

    def test_everything_is_created_the_first_time(self):
        p = sp.plan(manifest(), {}, ["Tree", "Bush"])
        self.assertEqual([i.name for i in p.create], ["Tree_0000", "Tree_0001", "Tree_0002"])
        self.assertEqual((p.update, p.hide, p.unhide, p.retarget), ([], [], [], []))

    def test_an_unchanged_manifest_does_nothing(self):
        m = manifest()
        p = sp.plan(m, have(m), ["Tree", "Bush"])
        self.assertEqual((p.create, p.update, p.hide, p.unhide, p.retarget, p.reparent), ([], [], [], [], [], []))

    def test_moving_a_point_updates_only_that_object_and_tiny_noise_does_not(self):
        m = manifest()
        ex = have(m)
        m2 = manifest(loc=[[0.0, 0.0, 0.0], [1.0, 0.000001, 0.0], [2.0, 5.0, 0.0]])
        p = sp.plan(m2, ex, ["Tree"])
        self.assertEqual([i.ident for i in p.update], [2])                                # 1e-6 is below the write threshold
        self.assertEqual(p.create, [])

    def test_fewer_points_hide_the_surplus_objects_and_more_points_reuse_them(self):
        m3 = manifest(ids=(0, 1, 2))
        m1 = manifest(ids=(0,))
        p = sp.plan(m1, have(m3), ["Tree"])
        self.assertEqual(sorted(p.hide), [1, 2])                                          # hidden, never deleted by a cook
        ex = have(m3, o1={"hidden": True}, o2={"hidden": True})
        p = sp.plan(m3, ex, ["Tree"])
        self.assertEqual(sorted(p.unhide), [1, 2])                                        # the same objects come back
        self.assertEqual(p.create, [])
        self.assertEqual(sorted(i.ident for i in p.update), [1, 2])
        # already hidden objects are not "hidden" again
        p = sp.plan(m1, have(m3, o1={"hidden": True}, o2={"hidden": True}), ["Tree"])
        self.assertEqual(p.hide, [])

    def test_sparse_stable_ids_keep_their_objects_when_others_come_and_go(self):
        m = manifest(ids=(10, 20, 30))
        ex = have(m)
        p = sp.plan(manifest(ids=(10, 30, 40)), ex, ["Tree"])
        self.assertEqual([i.ident for i in p.create], [40])
        self.assertEqual(p.hide, [20])
        self.assertEqual(p.update, [])                                                    # 10 and 30 stay put

    def test_changing_which_object_a_point_duplicates_retargets_it(self):
        m = manifest(src=[0, 0, 0])
        ex = have(m)
        p = sp.plan(manifest(src=[0, 1, 0]), ex, ["Tree", "Bush"])
        self.assertEqual([i.ident for i in p.retarget], [1])
        self.assertEqual(p.retarget[0].src, "Bush")

    def test_missing_sources_are_reported_once_and_a_placeholder_is_fixed_when_it_appears(self):
        m = manifest(src=[0, 1, 1])
        p = sp.plan(m, {}, ["Tree"])
        self.assertEqual(p.missing_sources, ["Bush"])
        ex = have(m, o1={"placeholder": True}, o2={"placeholder": True})
        p = sp.plan(m, ex, ["Tree", "Bush"])
        self.assertEqual(sorted(i.ident for i in p.retarget), [1, 2])
        p = sp.plan(m, ex, ["Tree"])
        self.assertEqual(p.retarget, [])                                                  # still missing: nothing new to do

    def test_parenting_to_the_owner_is_planned_when_it_differs(self):
        m = manifest()
        ex = have(m)
        p = sp.plan(m, ex, ["Tree"], parent="Host")
        self.assertEqual(sorted(p.reparent), [0, 1, 2])
        for e in ex.values():
            e.parent = "Host"
        self.assertEqual(sp.plan(m, ex, ["Tree"], parent="Host").reparent, [])

    def test_copies_carrying_a_used_id_are_hidden(self):
        m = manifest()
        extra = sp.Existing(1, "Tree_0001.001", "Tree")
        p = sp.plan(m, have(m), ["Tree"], extras=[extra])
        self.assertEqual(p.duplicates, ["Tree_0001.001"])

    def test_a_repeated_id_in_the_manifest_makes_one_object(self):
        m = manifest(ids=(1, 1, 2))
        self.assertEqual([i.ident for i in sp.items(m)], [1, 2])


if __name__ == "__main__":
    unittest.main()


class ShapedModeTests(unittest.TestCase):
    def test_curves_and_points_are_one_object_where_the_manifest_puts_it(self):
        for mode in ("curves", "points", "pieces"):
            m = {"type": "scene_objects", "node": "so", "key": "k", "prefix": "Strand", "mode": mode, "sources": [], "id": [0],
                 "loc": [[0.0, 0.0, 0.0]]}
            [item] = sp.items(m)
            self.assertEqual((item.name, item.src, tuple(item.scale)), ("Strand_0000", "", (1.0, 1.0, 1.0)))
            plan = sp.plan(m, {}, [])
            self.assertEqual([i.ident for i in plan.create], [0])

    def test_a_second_cook_updates_nothing_and_a_missing_id_parks_the_object(self):
        m = {"type": "scene_objects", "node": "so", "key": "k", "prefix": "S", "mode": "curves", "sources": [], "id": [0], "loc": [[0, 0, 0]]}
        have = {0: sp.Existing(0, "S_0000", "", False, False, (0, 0, 0), (0, 0, 0), (1, 1, 1), "")}
        self.assertEqual(sp.plan(m, have, []).update, [])
        gone = dict(m, id=[])
        self.assertEqual(sp.plan(gone, have, []).hide, [0])
