# Last Python node ports to Rust

Worktree: `bob/last-nodes`. Keep this file with each node commit. The Python node registry is the reference.

| Node | Status | Validation and differences | Open questions |
| --- | --- | --- | --- |
| `rbd_material_fracture` | Partial | Rust registry, shelf marker, all Python parameter names/defaults, first-level and repeated Voronoi fracture, assemble, glue/constraints, material RBD attributes. A two-piece box golden matches Python point/face counts, piece labels and masses; 1 focused Rust test. `convex_parts` and `interior_detail` and their controls explicitly reject non-default values because the required hull splitting and render-face tessellation are not yet ported. Native `rbd_solver` remains different from Python MuJoCo. | Complete and compare compound hull and interior-detail branches after convex decomposition exists. |
| `convex_decomposition` | Pending | | |
| `repeat` | Pending | | |
| `rectangle` | Pending | | |
| `polygon` | Pending | | |
| `python` | Pending | | |

Metadata generation and `tests.test_rust_param_meta`: 1 passed after the material fracture addition. Final workspace test totals pending.
