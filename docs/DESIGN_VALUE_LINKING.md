# Value linking and Ctrl+G grouping - design

Design by agy Argon, 2026-10-16. Not implemented; not checked by hand.

### 1. Existing State (Python & Rust)
- **Python (`minihou/`)**:
  - **Params & Expressions**: [`NodeInstance.params`](../minihou/graph.py#L46-L56) stores plain literals (`int`, `float`, `bool`, `str`, `vec3`). No `ch()` or inter-param links exist. [`timectx.py`](../minihou/timectx.py#L64-L81) detects `@Frame`/`@Time`/`@TimeInc` in string params.
  - **Grouping / Systems / Boxes**: `Ctrl+G` in [`index.html`](../minihou/web/static/index.html#L331-L335) calls `make_system`, using [`subgraph.analyse`](../minihou/subgraph.py#L65-L120), [`extract`](../minihou/subgraph.py#L123-L189), and [`replace_ops`](../minihou/subgraph.py#L192-L246) to save `nodes/<name>.json` loaded by [`_load_systems`](../minihou/library.py#L118-L212) with `input` port nodes and top-level `"exposed"` params. [`Graph.boxes`](../minihou/graph.py#L94-L126) + [`op_box_add`](../minihou/edit.py#L221-L263) handle frames (`O`) and sticky notes (`note: true`). [`nodes/group.py`](../minihou/nodes/group.py#L1-L40) is SOP geometry grouping.
- **Rust (`rust/crates/`)**:
  - **Core**: [`NodeDef`](../rust/crates/minihou-core/src/graph.rs#L35-L45) holds `params: Option<Map<String, Value>>`. [`Params::resolve`](../rust/crates/minihou-core/src/params.rs#L115-L148) strictly validates types against [`ParamSpec`](../rust/crates/minihou-core/src/params.rs#L48-L62). [`scalar_expr::Expr`](../rust/crates/minihou-core/src/nodes/scalar_expr.rs#L14-L45) already parses/evaluates closed math ASTs (`sin`, `cos`, `fit`, `clamp`, `smoothstep`, `@attr`, variables via callback). [`Session`](../rust/crates/minihou-core/src/session.rs#L22-L120) computes Merkle `u64` keys via [`key_of`](../rust/crates/minihou-core/src/session.rs#L130-L185).
  - **App & UI**: [`Document`](../rust/crates/minihou-app/src/lib.rs#L103-L195) does span-based lossless JSON edits (`Edit::Param`), top-level `"exposed"` toggling ([`toggle_exposed`](../rust/crates/minihou-app/src/lib.rs#L397-L449)), and Blender bridge syncing ([`link_param_edit`](../rust/crates/minihou-app/src/lib.rs#L529-L565)). [`ui/params.rs`](../rust/crates/minihou-app/src/ui/params.rs#L26-L185) renders widgets + reset/expose icons. [`ui/network.rs`](../rust/crates/minihou-app/src/ui/network.rs#L219-L252) already supports nested [`crumbs`](../rust/crates/minihou-app/src/ui/network.rs#L219-L237), and [`Keymap::blender`](../rust/crates/minihou-app/src/ui/keymap.rs#L102-L154) binds `O` to [`Command::NetworkBox`](../rust/crates/minihou-app/src/ui/keymap.rs#L23-L45).

### 2. Backwards-Compatible Graph JSON Data Model
Keep `"params"` storing last-evaluated fallback literals so older builds, [`default_graph.json`](../rust/crates/minihou-app/src/default_graph.json), and strict numeric readers never break. Add optional per-node `"channels"` and `"locked_params"` fields, plus `"subgraph"` for inline `subnet` nodes:
```json
"xform1": {
  "type": "transform",
  "params": { "translate": [0.0, 2.5, 0.0], "uniform_scale": 1.0 },
  "channels": {
    "translate.y": "ch('../box1/size.y') * 0.5 + @Time",
    "uniform_scale": "ch('../grid1/rows') / 10"
  },
  "locked_params": ["uniform_scale"]
}
```
- **Component addressing**: Scalar params use `"param"`, `Vec3` components use `"param.x"`, `"param.y"`, `"param.z"` (or whole-vector `"param"` for `chv()`).
- **Subnets**: A node of `"type": "subnet"` either embeds `"subgraph": { "nodes": {...}, "connections": [...], "exposed": [...] }` inline in the scene JSON or references `"system": "name"` (`nodes/<name>.json`).

### 3. Expression Grammar, Evaluation Order & Cycle Handling
- **Grammar**: Extend [`scalar_expr::Expr`](../rust/crates/minihou-core/src/nodes/scalar_expr.rs#L14-L45) with string literals in call args and built-ins:
  - `ch("../node/param[.x|y|z]")` / `ch("./param")` (numeric/bool coerced to `f64`/`i64`/`bool`), `chs("../node/param")` (string), `chv("../node/param")` (`[f64; 3]`).
  - Time vars `@Frame`, `@Time`, `@TimeInc` (also `$F`, `$T` aliases) resolved from [`TimeCtx`](../rust/crates/minihou-core/src/timectx.rs#L1-L40).
- **Evaluation Order (Decoupled from SOP Geometry Streams)**:
  - Parameter evaluation forms a **Channel Dependency DAG** over `(node_path, param_comp)` pairs, evaluated *before* SOP geometry cooking.
  - Evaluating `ch('../box1/size.y')` only resolves `(box1, size.y)`'s parameter value/expression—it **never** cooks `box1`'s geometry. Cross-stream references are $O(1)$ per channel hop.
- **Cycle & Error Handling**:
  - Maintain a `SmallVec<[(NodeId, ParamComp); 16]>` active stack during recursive resolution.
  - On cycle (`A.p -> B.q -> A.p`) or missing path, stop recursion, emit a structured [`ParamDiag`](../rust/crates/minihou-core/src/graph.rs) warning on the target param (`CycleDetected` / `BadReference`), and fall back to the literal value in `"params"`.

### 4. Cross-Stream Reference Resolution on Graph Edits
- **Path Syntax**: Houdini-standard relative paths (`../sibling/param`, `./other_param`, `../subnet1/inner/param`, `../../parent_node/param`) and root paths (`/node/param`).
- **Rename (`Edit::Rename`)**: AST walker rewrites all `ch/chs/chv` string literals across the graph (and inside subnets) referencing the old name to the new name within the same [`History`](../rust/crates/minihou-app/src/lib.rs#L260-L315) transaction.
- **Duplicate / Copy-Paste ([`graph_edit::copy`](../rust/crates/minihou-app/src/graph_edit.rs#L215-L270), [`paste`](../rust/crates/minihou-app/src/graph_edit.rs#L272-L345))**:
  - If **both** source and target are in the copied selection, rewrite relative `../old_src/p` to `../new_src/p` using the paste rename map.
  - If **only** the target is copied/duplicated, keep `../external_src/p` intact so the copy still links to the original driver.
- **Delete ([`graph_edit::delete`](../rust/crates/minihou-app/src/graph_edit.rs#L90-L150))**: Keep the expression on dependent nodes (Houdini parity), surfacing a broken-ref badge in UI while evaluating to the cached literal in `"params"`, so undoing deletion or recreating the node restores the link.

### 5. Cook Cache (`Session`) & Rust Engine Interaction
- In [`Session::key_of`](../rust/crates/minihou-core/src/session.rs#L130-L185), hash the **resolved** [`Params`](../rust/crates/minihou-core/src/params.rs#L115-L148) map (after evaluating `"channels"`) rather than raw `def.params`.
- If a driving parameter on stream A changes, stream B's resolved param value changes, automatically invalidating stream B's Merkle key without touching unrelated nodes.
- [`node_is_time_dependent`](../rust/crates/minihou-core/src/session.rs#L187-L220) checks if any expression in the node's transitive channel cone references `@Frame`/`@Time`/`@TimeInc`, folding [`timectx::key`](../rust/crates/minihou-core/src/timectx.rs#L25-L35) into `key_of`.

### 6. UI: Context Menu, Gestures, Link Visualisation & Keymap
- **Right-Click Param Context Menu ([`ui/params.rs`](../rust/crates/minihou-app/src/ui/params.rs))**:
  1. *Revert to Previous Value* (restores pre-session/undo value)
  2. *Revert to Defaults* (resets literal to [`ParamSpec::default`](../rust/crates/minihou-core/src/params.rs#L55) and clears channel)
  3. *Copy Parameter* (`Ctrl+Shift+C` on hovered row — copies `(node_path, param_comp, evaluated_value, expr)` to app clipboard)
  4. *Paste Values* (pastes literal value)
  5. *Paste Expressions* (pastes raw expression string)
  6. *Paste Relative References* (`Ctrl+Shift+V` — inserts `ch("../src/param")` or `chv(...)`)
  7. *Expression > Edit Expression (`Alt+E`) / Toggle Expression View*
  8. *Channels & Keyframes > Set Keyframe (`K`, writes `fit(@Frame, ...)` stub) / Delete Channels (`Ctrl+Shift+L`, bakes current evaluated value into `"params"` and removes `"channels"` entry)*
  9. *Reference > Go to Source (`[`) / Go to Target(s) (`]` submenu)*
  10. *Lock / Unlock Parameter* (toggles membership in `"locked_params"`, disabling edits/drags)
- **Widget & Drag-to-Link Gesture**:
  - Linked params render with a **cyan/green tint border** and `=`/`fx` badge; clicking toggles between **evaluated number** and **expression string**.
  - **Alt-drag** from a param label/icon onto another param row (or onto a node card in [`ui/network.rs`](../rust/crates/minihou-app/src/ui/network.rs) to open a param picker popup) creates a `ch("../src/param")` link.
- **Network Editor Link Visualisation ([`ui/network.rs`](../rust/crates/minihou-app/src/ui/network.rs))**:
  - Draw **dashed cyan/purple cubic Beziers** (`#38bdf8` valid, `#ef4444` cycle/broken) horizontally between node card sides (`card_rect.right_center()` to `target_rect.left_center()`), distinct from vertical SOP wires.
  - Render a small link badge (`🔗 N`) on cards with incoming/outgoing refs. Dim links (`alpha = 0.25`) by default; highlight (`alpha = 0.95`, animated dash + tooltip `src.param -> dst.param`) when either endpoint node/param is selected or hovered. Toggle all link lines with `Shift+D`.
- **Keymap Impact ([`ui/keymap.rs`](../rust/crates/minihou-app/src/ui/keymap.rs))**:
  - Add in `Context::Network`: `Ctrl+G` (`GroupSubnet`), `Alt+G` (`UngroupSubnet`), `Enter` (`DiveInto`), `U` / `Alt+Up` (`JumpUp`), `Shift+D` (`ToggleDepLinks`), `Alt+N` (`StickyNote` alongside existing `O` `NetworkBox`).

### 7. `Ctrl+G` Group Semantics: Hybrid Inline Subnet + Library Promotion
- **Decision**: **Houdini-style Subnetwork (`type: "subnet"`)** stored inline in the graph's `"subgraph"` field by default, with a 1-click *"Save as Library System..."* button in its parameter header to promote it to `nodes/<name>.json` (unifying with Python's [`subgraph.py`](../minihou/subgraph.py) and [`library.py`](../minihou/library.py)).
- **Inside the Subnet**:
  - [`subgraph::extract`](../minihou/subgraph.py#L123-L189) ports to Rust: external incoming wires become `input` nodes (`index: 0..k`, max 4 ports on the outer card); the selected display/terminal node feeds an `output` node (`type: "output"`).
  - Double-clicking or pressing `Enter` pushes the subnet name onto [`network::State::nested`](../rust/crates/minihou-app/src/ui/network.rs#L219-L252) (already plumbed in [`crumbs()`](../rust/crates/minihou-app/src/ui/network.rs#L219-L237)); `U` / `Alt+Up` pops back out.
- **How Cooking & Channels See It**:
  - [`Session`](../rust/crates/minihou-core/src/session.rs) recursively cooks `"subgraph"`, binding outer input [`Arc<Geometry>`](../rust/crates/minihou-core/src/geo.rs) streams to inner `input` nodes and returning the inner `output`/`display` geometry.
  - Inner parameters exposed via `"exposed"` appear on the outer `subnet` node's parameter pane and can be driven by outer `ch()` expressions; inner nodes can also reference outer params via `ch("../../node/param")`.
  - Also complete rendering for existing `"boxes"` (`O` for frames, `Alt+N` for sticky notes) in [`ui/network.rs`](../rust/crates/minihou-app/src/ui/network.rs).

### 8. Staged Implementation Plan (6 Steps)
| Step | Size | Scope & Key Details | Files Touched | Parallel? |
| :--- | :---: | :--- | :--- | :---: |
| **1. Core Channel AST & Schema** | **M** | Add `channels` & `locked_params` to [`NodeDef`](../rust/crates/minihou-core/src/graph.rs#L35-L45); extend [`scalar_expr.rs`](../rust/crates/minihou-core/src/nodes/scalar_expr.rs) with `ch`/`chs`/`chv` & `@Frame`/`@Time`; implement DAG resolver with cycle detection & fallback to literals. | [`graph.rs`](../rust/crates/minihou-core/src/graph.rs), [`params.rs`](../rust/crates/minihou-core/src/params.rs), [`scalar_expr.rs`](../rust/crates/minihou-core/src/nodes/scalar_expr.rs) | Starts first (parallel with **4** & **5**) |
| **2. Session Cache & Ref Rewriting** | **M** | Feed resolved channel values & transitive time-dep into [`Session::key_of`](../rust/crates/minihou-core/src/session.rs#L130-L185); add `Edit::Channel` & `Edit::LockParam`; rewrite `ch()` paths on rename/copy/paste in [`graph_edit.rs`](../rust/crates/minihou-app/src/graph_edit.rs). | [`session.rs`](../rust/crates/minihou-core/src/session.rs), [`lib.rs`](../rust/crates/minihou-app/src/lib.rs), [`graph_edit.rs`](../rust/crates/minihou-app/src/graph_edit.rs) | After **1** (parallel with **3**, **4**, **5**) |
| **3. Params UI, Menu & Drag-Link** | **M** | Add Houdini right-click menu (Copy/Paste Rel Refs, Revert, Delete Channels, Lock, Go-to), expression/value toggle, cyan link styling, and `Alt`-drag linking in [`ui/params.rs`](../rust/crates/minihou-app/src/ui/params.rs). | [`ui/params.rs`](../rust/crates/minihou-app/src/ui/params.rs), [`ui/mod.rs`](../rust/crates/minihou-app/src/ui/mod.rs), [`main.rs`](../rust/crates/minihou-app/src/main.rs) | After **1** (parallel with **2**, **4**, **5**) |
| **4. Network Link Visualisation** | **S** | Render dashed dependency Beziers, card `🔗` badges, hover/selection highlighting, `Shift+D` toggle, and `[`/`]` Go-to navigation in [`ui/network.rs`](../rust/crates/minihou-app/src/ui/network.rs). | [`ui/network.rs`](../rust/crates/minihou-app/src/ui/network.rs), [`ui/keymap.rs`](../rust/crates/minihou-app/src/ui/keymap.rs) | Runs in parallel with **1–3** |
| **5. Subnet Engine & `Ctrl+G` Extract** | **L** | Add `subnet`, `input`, `output` node specs and recursive subgraph cooking in `minihou-core`; port [`subgraph.py`](../minihou/subgraph.py) extraction/ungroup (`Ctrl+G`/`Alt+G`) to [`graph_edit.rs`](../rust/crates/minihou-app/src/graph_edit.rs). | [`graph.rs`](../rust/crates/minihou-core/src/graph.rs), [`session.rs`](../rust/crates/minihou-core/src/session.rs), [`graph_edit.rs`](../rust/crates/minihou-app/src/graph_edit.rs) | Runs in parallel with **1–4** |
| **6. Subnet Navigation & Boxes UI** | **M** | Wire nested breadcrumb diving (`Enter`/`U`/double-click), library promotion (`nodes/<name>.json`), and `"boxes"` / sticky note canvas rendering in [`ui/network.rs`](../rust/crates/minihou-app/src/ui/network.rs). | [`ui/network.rs`](../rust/crates/minihou-app/src/ui/network.rs), [`ui/keymap.rs`](../rust/crates/minihou-app/src/ui/keymap.rs), [`main.rs`](../rust/crates/minihou-app/src/main.rs) | After **4** & **5** |
