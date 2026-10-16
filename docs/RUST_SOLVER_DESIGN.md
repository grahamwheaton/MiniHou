# Rust solver / physics port: design note

Scope: port the Python time-dependent and physics nodes to `minihou-core`:
`goal_length`, `goal_collision`, `goal_surface`, `goal_gravity`, `goal_pin`, `goal_solve` (procedural_systems.py),
`solver` (nodes/solver.py), `rbd_configure`, `rbd_constraint`, `rbd_solver` (nodes/rbd.py + minihou/rbd.py),
`rbd_constraint_properties`, `rbd_material_fracture` (nodes/fracture_tools.py). Python stays the reference.

## 1. Time context (`minihou-core/src/timectx.rs`)

Mirror of `minihou/timectx.py`. Python keeps `(frame, fps)` and an optional step (`@TimeInc`) in context variables;
Rust keeps them in a **thread-local** with RAII guards:

* `timectx::frame()`, `fps()`, `seconds()` (`(frame-1)/fps`), `time_inc()` (step, else `1/fps`).
* `timectx::at(frame, fps, || ...)` / `timectx::step(dt, || ...)`: scoped overrides, restored on exit (also on panic,
  via a drop guard).
* `timectx::key()`: cache-key text `t{frame}@{fps}/{step}` as in Python.
* `timectx::params_use_time(params)`: a string parameter mentions `@Frame`, `@Time` or `@TimeInc`, or `animated` is true.

Caveat: thread-local, so nodes must read the frame on the cooking thread (before any rayon fan-out).

## 2. Session / graph plumbing

* `Session` gains `set_time(frame, fps)` / `time()`. `Session::cook` (and the key queries `side_payload`,
  `cached_geometry`) run inside `timectx::at(frame, fps)`. Default frame 1, 24 fps (Python's default).
* `key_of` hashes `timectx::key()` into a node's content key when the node is time dependent: its type is in
  `nodes::TIME_DEPENDENT` (`solver`, `rbd_solver`) or its parameters use time (`params_use_time`). Downstream keys change
  through their input keys, exactly like Python's `key_extra`. Scrubbing therefore recooks only time-dependent branches.
* `Graph::cook` needs no change: it reads the thread-local; callers wrap it in `timectx::at` to cook at a frame.
* App: `minihou_app::Edit::Time { frame, fps }` goes through the cook worker like any edit and calls
  `session.set_time` (not a document edit: no undo entry, not saved). Blender's `set_frame` sends it (main.rs). The
  in-app timeline (`ui/viewport_ui.rs` `VpState.frame`) is not wired yet: it needs a small accessor in that UI file and
  an `Edit::Time` send in main.rs when the frame changes (left to the UI owners).
* `Graph::cook` / `Session::cook` refuse time-dependent nodes upstream of an `rbd_solver`
  (`rbd_engine::check_static_inputs`, Python's evaluator check, same message).

## 3. Solver node (`nodes/solver.rs`)

`solver(geo; body, params, start_frame, substeps)`: runs node type `body` on the previous frame's result from
`start_frame` to `round(frame)`, `substeps` per frame, each substep inside `timectx::at(f-1+(k+1)/substeps)` and
`timectx::step(1/(fps*substeps))`. The body's params come from the JSON string (`Params::resolve`, which also rejects
unknown names). Bodies whose other inputs are required get nothing connected (as Python passes `None`).

Frame memo (Python: a WeakKeyDictionary keyed by input geometry): a process-global `Mutex` map keyed by the input
`Arc<Geometry>` identity (pointer + a `Weak` to prove it is still the same allocation) and the settings text; each holds
up to 1000 frames (oldest dropped). Playback steps from the nearest remembered frame <= target; dead inputs are pruned.
Results are identical however a frame is reached (pure function of input + settings + frame).

## 4. Geometric goals (`nodes/goals.rs`)

Goals are stored exactly as in Python: detail `geometric_goals` (list of JSON goal dicts) and
`geometric_goal_topology = [npoints, topology digest]`. The digest is Python's `blake2b(digest_size=16)` over
`vertex_point` (int32 LE) | `prim_offsets` | (`|open|` + closed bytes when any primitive is open), so a Rust and a
Python chain write the same detail. A small BLAKE2b is implemented locally (no new crate dependency).
`goal_solve` is a straight port of the position-based relaxation (float64 throughout, same accumulation order:
`np.add.at` order, degree normalisation, pin enforcement, residuals). `goal_surface` needs closest point on triangles
(Ericson 5.1.5, same formula as `raycast._closest_on_tri`); a small AABB tree replaces `TriGrid` (exact nearest, so the
same answer except for exact ties).

## 5. RBD family

* `rbd_configure`, `rbd_constraint`: attribute writers, direct ports.
* `rbd_constraint_properties`: direct port; `variation` needs NumPy's `default_rng([seed, 53]).random(n)`, so a
  NumPy-compatible SeedSequence + PCG64 + `random()` is implemented (`nodes/np_random.rs`) and checked against values
  produced by NumPy.
* `rbd_solver`: the Python node is a thin wrapper around MuJoCo 3.15.0 (`minihou/rbd.py`), an optional dependency. There
  is no MuJoCo in the Rust build and bit-parity with MuJoCo is impossible without it. The Rust node validates inputs
  exactly like `rbd.pieces` / `link_table` and simulates with a small native engine (see `nodes/rbd_solver.rs`
  header for what it does and does not model). Cache: worlds keyed by a content digest of inputs + settings + fps
  (Python keys per node id + digest), at most 8 resident, each with a byte-bounded pose cache (`cache_mb`); playback
  steps forward, going back restarts from the initial state (never resumes from a snapshot), as in Python.
* `rbd_material_fracture`: a composition of `voronoi_fracture_points`, `voronoi_fracture`, `assemble`, `glue_cluster`,
  `convex_decomposition`, `concave_volume`, `mesh_volume`, `tessellate` + `fbm3` (interior detail). **Not ported yet**:
  `voronoi_fracture_points`, `glue_cluster` and `convex_decomposition` are not in the Rust registry. Once they are, the
  node is a direct composition (its own code is ~90 lines of glue around them) and belongs in `nodes/rbd_fracture_tools.rs`.

Native solver status (tests in `rbd_engine.rs`, mirroring `tests/test_rbd.py` / `test_rbd_constraints.py`): drop onto a
floor rests within (-0.005, 0.015); bodies stay rigid; frame-by-frame equals direct jump, scrub back replays from the
start, fractional frames hold; brick wall knocked down by a projectile while an unshot wall stays upright; damping and
mass; bounce; spring settles at 1 m + 9.81/100; hinge stops at +35 degrees; pin and glue hold, break above Strength
(after the 0.1 s settle time) and pieces stay glued to each other.

## 6. Tests

* Unit tests per file; Python-oracle fixtures in `nodes/test_data/solver_parity.json`, generated by
  `tools/dump_rust_solver_fixtures.py` (graph JSON + frame -> encoded Python result).
* Session tests: scrub forward/back gives identical results; only time-dependent nodes recook when the frame changes.
