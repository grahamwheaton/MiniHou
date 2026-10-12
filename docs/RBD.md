# Rigid bodies: first working stage

RBD Configure and RBD Solver run actual convex rigid-body contacts in the **external
MiniHou engine**, using official **MuJoCo 3.15.0**. Blender displays the resulting
mesh. No physics dependency is installed into Blender. This is not full Houdini parity.

## Start

With a normal Python 3.12 virtual environment, install `pip install ".[physics]"`.
The Windows portable package includes a separate CPython 3.12.10 runtime with
MuJoCo 3.15.0 and NumPy 2.2.6. Extract it, then run `Start-RBD.cmd`. It uses link
port 8875 and editor port 8876; change those if occupied. The runtime's `_pth`
uses a relative path to this extracted package, never the original MiniHou checkout.

Install `MiniHou-Link-RBD.zip` in Blender. In the add-on preferences select the
extracted project folder and `build/runtime/python312/python.exe`. Set the link
port to 8875 when using Start-RBD.cmd. Either use that launcher or the add-on's
existing Start Server controls; do not start two engines on the same port.

For the demos, create an **empty mesh object** (delete all vertices in Edit Mode),
then add a MiniHou modifier using `examples/rbd_wall_modifier.json` or
`examples/rbd_falling_modifier.json`. The normal Geometry Nodes modifier panel
exposes velocity, mass, gravity, substeps and reset. Start at frame 1, 24 FPS,
and play through frame 96. For an existing source mesh, connect Modifier Input
directly to RBD Configure, then RBD Solver. The demo wrapper retains its host
input after the simulation, so a nonempty host will also remain visible.

The standalone/editor versions are `examples/rbd_wall.json` and
`examples/rbd_falling.json`. Graph changes from the editor are adopted into an
existing modifier with the existing Reload Graph action. Exposed modifier controls
take effect directly. The editor shows the new nodes under Physics and provides a
Reset Simulation button on RBD Solver.

## Network and data

```
Mesh -> Fracture -> RBD Configure -----------+
             ^                              +-> RBD Solver -> Output
     Scatter(volume)     Box -> Configure --+

Brick -> Copy Transform(row) -> Copy Transform(rows) -> Configure --+
Sphere -> Configure(initial velocity, mass, unique ID offset) ------+-> Merge -> Solver
Floor box ---------------------------------------------------------------> Solver.colliders
```

Configure selects primitive IDs from `piece_attrib` (e.g. fracture's `piece`), or
uses connectivity when empty. It writes scalar primitive `rbd_id`, `rbd_active`,
`rbd_mass`, `rbd_friction`, `rbd_restitution`, `rbd_linear_damping`,
`rbd_angular_damping`, and vector `rbd_v`/`rbd_w`. A primitive wrangle can override
these, but every face of a piece must agree. Configure separate branches with
different `id_offset` values before merging them. IDs do not need to be contiguous.
Colliders are always static; unconfigured collider meshes use default properties.

Positions already contain initial rotation/scale/translation: put Transform before
Configure. The backend uses a convex hull for collision and mass/inertia, with
uniform density and the configured mass per body. The original rendered geometry
never passes through a mesh remesher. Topology, point/corner/primitive ordering,
UVs, materials, piece attributes and groups are retained; normals in all three
domains rotate with their bodies. `rbd_v`/`rbd_w` on solver output contain current
body velocities. `detail.rbd` reports backend, version, frame and body count.

Both inputs share one **simulation coordinate space**. A modifier uses object-local
coordinates, including local gravity and floor. One unit represents one metre;
velocity is units/sec, angular velocity radians/sec, damping exponential decay/sec.
Use applied/identity object transforms for predictable world-scale physics.

MuJoCo uses soft contacts. Restitution maps to the contact damping ratio of an
ideal damped oscillator, clamped for stability; it is an approximate bounce
control, not an exact Newton restitution coefficient. Contact parameters mix
between the two bodies, so set floor bounce as well when testing bounce. Friction
uses MuJoCo's contact mixing. Margin is per shape: two margins can leave a small
visible resting gap. Increase substeps for smaller/faster bodies; there is no CCD
guarantee. The brick demo uses 12 substeps and a large floor so the projectile
does not leave it during the demonstration.

## Time, reset and ownership

Frame 1 is the default initial state. Frame/FPS come from the editor or Blender;
the solver uses fixed `1/(FPS*substeps)` steps. Frames at/before start show the
initial pose. Fractional frames hold the preceding integer frame (no motion blur).
Change Reset to invalidate and replay. Physics input geometry, attributes,
colliders, settings, FPS and backend version all participate in invalidation.
Editing a static input rebuilds the simulation; animated input is not a supported
way to drive it.

Each evaluator/modifier instance owns its world. A removed modifier, lost connection,
session stop or explicit evaluator `close()` releases it. There are at most eight
resident solver worlds per evaluator; least recently used worlds are released.
Each world has a bounded pose cache (32 MiB default, configurable 1..1024 MiB).
The normal geometry cache has its existing independent byte budget. Physics inputs
are limited to 2000 bodies and 200000 collision vertices.

Cached poses are display snapshots, not restart checkpoints. A backward request
outside the pose cache resets all MuJoCo state and replays from the start; subsequent
forward steps are incremental. This preserves contact/warmstart state correctly.
Repeatability is tested for the pinned build on the same platform; no cross-version
or cross-platform bitwise guarantee is made. Large uncached jumps may take time.

## Blender playback and rendering

RBD modifiers synchronously cook on Blender's frame-change handler. Actual
`animation_play` and evaluated GN output were tested in Blender 5.2. This can play
slower than real time for expensive scenes; use Play Every Frame for inspection.
The original legacy proxy output remains eventual; use the modifier workflow for
the tested playback/render path.

For a scripted still render, call `minihou_link.modifiers.cook_frame(scene, frame)`
before `bpy.ops.render.render(write_still=True)`. It raises if cooking fails.
Do not render an unbaked animation in one render call: the live timer is suspended
during rendering. Either explicitly cook/render each still in sequence, or use
**Bake RBD Frame Range** in the modifier-properties MiniHou panel.

The bake creates a new collection of per-frame evaluated meshes with native
constant scale keys (zero for inactive samples). It retains material slots, UVs, groups and evaluated normals,
hides the source only after success, and leaves its graph/modifiers intact. Failure
removes only this attempted bake. The bake needs no engine, add-on or Python handlers
to render. The supplied baked .blend demos contain frames 1..96. Native background
animation rendering was tested with the engine disconnected. Unhide the source and
hide the bake collection to edit/rebake; existing bakes are never overwritten.

For reliable background rendering of an arbitrary subset, use
`tools/render_rbd_bake.py` with `--start`, `--end` and `--output` after Blender's
`--` separator. The helper evaluates the starting frame **before** constructing
the render depsgraph; this is necessary when jumping to a new range start in the
tested Blender build. It requires only Blender, not the engine or add-on. Full-range
rendering from the saved start frame also works with Blender's normal Render Animation.
The helper optionally compares a rendered frame against a live reference image.

This initial bake prioritizes data preservation over size: one mesh per frame,
maximum 1000 frames and a conservative 512 MiB geometry estimate by default.
Outside the baked range all samples are hidden. Subframe interpolation/motion blur
and transformed/animated source owners are not supported.

## Current limits

Closed, planar-faced, outward-wound **convex solid pieces** only, including the
static floor (use a box, not an open grid). Concave, non-manifold, inward-wound,
degenerate or shared-point pieces fail clearly. Do not assume every fracture of
a concave source is convex. No automatic convex decomposition, glue, constraints,
fracture-on-impact, joints, animated/kinematic colliders, deforming geometry,
changing topology, source-object visual instances or GPU simulation yet.

Time-dependent graph inputs, standard animated Blender objects/drivers/shape keys,
native simulation modifiers and time/simulation Geometry Nodes are rejected.
Arbitrary custom nodes must obey the static-input contract. RBD Solver must be
top-level; reusable systems can prepare/configure geometry but cannot own the
solver. This prevents shared library evaluators from sharing physics worlds.

Fracture's existing cap UV/material policy is unchanged: its new faces initially
zero-fill attributes. Assign the `inside` group an appropriate material before
solving when needed.

## Verification and backend choice

Official PyBullet release metadata was checked: no CPython 3.12 Windows wheel.
Official MuJoCo 3.15.0 supplies `cp312-win_amd64`; no third-party fork was used.
Sources: https://pypi.org/project/mujoco/3.15.0/,
https://pypi.org/project/pybullet/3.2.7/,
https://mujoco.readthedocs.io/en/stable/modeling.html#solver-parameters.

Run `python -m unittest tests.test_rbd tests.test_rbd_instances -v` with the physics
extra. Tests cover contacts, wall impact versus an undisturbed wall, angular motion,
mass/damping/bounce controls, rigidity, attributes/normals, replay/reset, FPS/settings/
collider invalidation, isolation and cleanup. `bench/bench_rbd.py` measures 25/100
bricks plus a projectile over 120 frames. This machine measured approximately
3.7/14.4 ms median engine cook per frame, excluding Blender transfer/draw.

`tools/blender_rbd_smoke.py` and `tools/blender_rbd_falling_smoke.py` test real Blender
with an owned external engine. Run with `--background --factory-startup
--python-exit-code 1 --python <script>`. Running the wall script without background,
with `-- --playback`, also exercises actual screen animation playback. All scripts
use this worktree's isolated runtime by default, or `MINIHOU_ENGINE_PYTHON` if supplied.
