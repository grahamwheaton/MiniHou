# Last Python node ports to Rust

Worktree: `bob/last-nodes`. Keep this file with each node commit. The Python node registry is the reference.

| Node | Status | Validation and differences | Open questions |
| --- | --- | --- | --- |
| `rbd_material_fracture` | Partial | Rust registry, shelf marker, all Python parameter names/defaults, repeated Voronoi fracture, assemble, glue/constraints, optional convex-part splitting, and material RBD attributes. A two-piece box golden matches Python point/face counts, piece labels and masses; 1 focused Rust test. `interior_detail` and its controls explicitly reject non-default values because render-face tessellation and seam rebuilding are not ported. Native `rbd_solver` remains different from Python MuJoCo. | Complete and compare interior-detail and compound-hull branches on production meshes. |
| `convex_decomposition` | Done, approximate | Pure Rust hull-volume-driven recursive plane cuts with the Python parameter defaults, `piece` attribute and four output groups. Golden convex box matches 8 points, 6 faces, one piece; an L prism splits into closed pieces with conserved volume. 2 focused Rust tests. The Rust plane search spreads face candidates and uses slightly shifted axis planes to avoid coincident-face clipping; cut order and piece counts can differ from Python's reflex-edge search. Connected components are not independently seeded before cuts. | Improve matching for disconnected inputs and difficult concave caps if production scenes require it. |
| `repeat` | Skipped | No Python or Rust node with this type name. The Python and Rust registries both contain the paired `loop_begin`/`loop_end` region, and Rust evaluates repeat regions through `nodes/loop.rs`. | None. |
| `rectangle` | Skipped | No Python node with this type name. Rectangle is a viewport drawing tool, not a graph node. | None. |
| `polygon` | Skipped | No Python node with this type name. Polygon is a viewport drawing tool, not a graph node. | None. |
| `python` | Explicit unsupported stub | Rust registry and shelf marker now expose all Python inputs and parameter defaults. The cook always reports that a Python runtime bridge is unavailable; 1 focused Rust test. `minihou-py` is a Python extension that calls Rust, not an embeddable interpreter. The Rust app has no Python node permission switch or full numeric/string geometry exchange. Python's own engine still executes trusted snippets only after explicit opt-in. | Decide whether a future app-level, opt-in subprocess bridge with a bounded runtime and complete geometry/attribute serialization is worth maintaining. |

Metadata generation and `tests.test_rust_param_meta`: 1 passed after the material fracture addition. Final workspace test totals pending.
