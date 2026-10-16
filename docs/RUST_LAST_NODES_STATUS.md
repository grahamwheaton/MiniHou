# Last Python node ports to Rust

Worktree: `bob/last-nodes`. Keep this file with each node commit. The Python node registry is the reference.

| Node | Status | Validation and differences | Open questions |
| --- | --- | --- | --- |
| `rbd_material_fracture` | Partial | Rust registry, shelf marker, all Python parameter names/defaults, first-level and repeated Voronoi fracture, assemble, glue/constraints, material RBD attributes. A two-piece box golden matches Python point/face counts, piece labels and masses; 1 focused Rust test. `convex_parts` and `interior_detail` and their controls explicitly reject non-default values because the required hull splitting and render-face tessellation are not yet ported. Native `rbd_solver` remains different from Python MuJoCo. | Complete and compare compound hull and interior-detail branches after convex decomposition exists. |
| `convex_decomposition` | Done, approximate | Pure Rust hull-volume-driven recursive plane cuts with the Python parameter defaults, `piece` attribute and four output groups. Golden convex box matches 8 points, 6 faces, one piece; an L prism splits into closed pieces with conserved volume. 2 focused Rust tests. The Rust plane search spreads face candidates and uses slightly shifted axis planes to avoid coincident-face clipping; cut order and piece counts can differ from Python's reflex-edge search. Connected components are not independently seeded before cuts. | Improve matching for disconnected inputs and difficult concave caps if production scenes require it. |
| `repeat` | Pending | | |
| `rectangle` | Pending | | |
| `polygon` | Pending | | |
| `python` | Pending | | |

Metadata generation and `tests.test_rust_param_meta`: 1 passed after the material fracture addition. Final workspace test totals pending.
