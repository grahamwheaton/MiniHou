# Migration of legacy Rust geometry nodes

## Scope and order

The package contract is `docs/NODE_DEVELOPMENT.md`: one geometry output, a
`NodeSpec`, `node.toml`, and package parameter metadata. Keep each existing ID,
input names, defaults, validation, and cook implementation. Move one node at a
time so a duplicate ID fails immediately. The following order starts with
independent nodes and postpones shared code and parity fixtures.

1. **Pilot:** `measure_volume`, `measure_bounds`, `measure_area`. These have a
   single geometry input, one parameter, local cook code and Rust unit tests.
   None appears in `tests/test_rust_parity.py` or the Rust parity modules.
2. **Remaining measurements:** `measure_length`, `measure_distance`. Their
   local tests cover empty and grouped input; distance has several selection
   modes, so move it after the simpler length node.
3. **Other isolated single-output nodes:** `null`, `output`, `input`, then
   independent generators and modifiers such as `point`, `line`, `bound`,
   `reverse`, `facet`, `peak`, and `set_material`. Audit callers of each module
   and Python parity coverage before moving it. `null` and `lod_bake` are
   declared inline in `nodes/mod.rs` and need their cook functions extracted.
4. **Shared-module families:** `generate` shapes, `curves`, `group`, `attribs`,
   `attrib_tools`, `model`, `modeling_tools`, `pcg`, and similar modules. Move
   each public `NodeSpec` independently while retaining shared helper modules
   until their last consumer is gone. Check test paths and private helper
   visibility for every extraction.
5. **Integration and parity-heavy nodes:** Blender/image/USD I/O, packed and
   instance operations, boolean/volume/fracture, and solver/RBD nodes. These
   can use packages only if their current `NodeSpec` fits the one-geometry-
   output contract. Preserve time-dependent IDs in `TIME_DEPENDENT`; rerun
   their Python and Rust oracle suites and any host integration tests. Defer
   nodes needing extra sockets, custom UI, or icons until package support
   covers them.

The web editor and Python registry have their own catalogues; package discovery
does not change them. A Rust node with no native shelf entry still needs an
intentional native category and label.

## Files touched for each move

| File | Change |
| --- | --- |
| `rust/crates/minihou-core/src/nodes/packages/<id>/node.rs`, `node.toml`, `tests.rs`, `param_meta.json` | Move the spec/cook/tests; declare ID, label, category, description, keywords, and the unchanged parameter widget metadata. |
| `rust/crates/minihou-core/src/nodes/mod.rs` | Remove the legacy registry entry and module declaration only when no other code uses the module. Update the fixed package-ID test to account for the added ID. |
| `tools/dump_rust_param_meta.py` | Remove handwritten Rust-only metadata for the moved ID (or its Python-derived `TYPES` entry). The package reader must be the sole metadata source. |
| `rust/crates/minihou-app/src/param_meta.json` | Regenerate after each move. Compare the moved entry before and after; type, range, options, doc and default must match. |
| `rust/crates/minihou-app/src/ui/shelf_data.rs` | Remove the native legacy tool to avoid a duplicate; the generated catalogue appends the package tool to its category with a generic icon. Check label and order changes. |
| `rust/crates/minihou-app/src/ui/network_menu_data.rs` | Remove the native legacy Tab entry if the Tab catalogue also appends package entries. Check its label and order changes. |

`network_menu_web.json` and `minihou/web/static/shelf.js` are web catalogue
data and stay in place for a Rust-only migration. A separate UI project may
unify ordering and restore custom icons; package metadata currently has no icon
field.

## Checks and risks

- The registry currently has a test asserting exactly three package IDs. Update
  it for each addition. Search for tests or consumers that address a removed
  module or assume legacy registry order or length.
- Existing node tests should move with the cook code. Run them by node ID, then
  the focused registry and native catalogue tests. Do not treat a compile-only
  check as cook-output verification.
- `tests/test_rust_param_meta.py` compares generated metadata with the checked
  in JSON. Run the generator and this test after each move. Metadata conflicts
  cause generation to fail if handwritten and package entries coexist.
- Python parity tests, Rust oracle fixtures, and tests that compare Python and
  Rust metadata may encode IDs, defaults, topology, or attributes. Audit them
  before selecting a node; run their focused cases for later parity-heavy
  moves. This pilot does not change Python node definitions.
- A package tool is appended after legacy tools and gets the generic native
  icon. Native shelf/Tab order and icon can visibly change even when cooking
  does not. Check the generated catalogue for one entry per ID.
