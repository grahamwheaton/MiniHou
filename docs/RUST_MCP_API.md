# Rust app HTTP API for the existing Python MCP server

Contract audited against `minihou/mcp.py`, `minihou/web/server.py`,
`minihou/link/session.py`, `minihou/edit.py`, `minihou/actions.py` and
`minihou/summary.py`. This checkout defines **14 MCP tools**, not 34.
The transport remains `tools/mcp_server.py` / `python -m minihou mcp`, stdio
JSON-RPC. Neither that server nor `minihou/mcp.py` needs a Rust-specific change.

The running native editor listens on **127.0.0.1:8766**. It serves JSON, not the
Python browser editor. The separate Blender link remains on 8765. A bind failure
is shown in the Blender/status area; it never terminates the editor or silently
chooses a different HTTP port. Close the competing engine and reopen the app.

## Exact HTTP calls made by MCP

`EngineClient` sends one HTTP/1.1 request per connection with `Host`, and for
POSTs `Content-Type: application/json` and a JSON object body. Its GET timeout is
60 seconds (the alive probe uses 2); POST timeout is 120 seconds. HTTP >=400 is a
tool error using the JSON `error` string. Invalid JSON responses are errors.
Every Rust reply has Content-Length and Connection: close. Bodies are limited to
8 MiB, headers to 16 KiB; chunked requests are not accepted. Bind and Host/Origin
checks restrict the service to local clients. No authentication is added.

In the table, `snapshot`, `status` and `node-type` have the shapes below. Optional
fields are marked `?`. GET query parameters are separate from JSON bodies.

| MCP tool | Method and path | Exact payload/query from mcp.py | Response consumed by mcp.py | Live status 2026-10-17 |
|---|---|---|---|---|
| `engine_status` | GET `/api/state` | none | snapshot plus `status`, `link:{clients:[str],port}`, `engine:{pid,version,python_nodes}`, `library?:{dirs:[str],files:[{file,nodes:[str],error:null\|str}]}` | Pass |
| `list_node_types` | GET `/api/nodes` | none; filtering `names` is done in the MCP process | `[node-type,...]` | Pass, `names` filter |
| `get_graph` | GET `/api/graph`, then GET `/api/status` | none; `include_layout=false` strips node `pos` in the MCP process | snapshot and status | Pass |
| `edit_graph` | POST `/api/edit` | `{"ops":[...],"wait":true,"timeout":60,"origin":"mcp"}` | `{"changed":bool,"rev":int,"results":[object,...],"status":status,"graph":snapshot}` | Pass, replace/add/connect/set/display |
| `inspect_node` | POST `/api/inspect` | `{"node":str,"detail":bool}` (tool default true) | inspection below; `ok=false` becomes a tool error | Pass |
| `undo` | POST `/api/undo` or `/api/redo`, then GET `/api/status?wait=1&timeout=30` | `{"origin":"mcp"}` once per step, clamped to 1..50 | `{"ok":bool,"graph":snapshot}` and status | Pass, undo and redo |
| `export_usd` | POST `/api/export_usd` | `{"path":str,"overwrite":bool,"graph":bool,"node"?:str}`; overwrite default false, graph default true | `{"path":str,"outputs":{name:{"prim":str,"faces":int,"curves":int,"points":int,"instances"?:int,"dropped":{"faces":int,"curves":int}}}}` | Pass, `.usda` file written |
| `press_button` | POST `/api/node_action` | `{"node":str,"action":str}` | object with optional `message` string | Not exercised live |
| `set_proxy` | GET `/api/proxy` if no args, otherwise POST `/api/proxy`, GET `/api/status?wait=1`, GET `/api/proxy` | POST contains only supplied fields: `level?:number`, `auto?:bool`, `link_tris?:int` | `{"level":number,"auto":bool,"cooked":number,"link_tris":int,...}` | Not exercised live |
| `set_frame` | POST `/api/frame`, GET `/api/status?wait=1` | `{"frame":number,"fps"?:number}` | `{"frame":number,"fps":number}` and status | Not exercised live |
| `list_graphs` | GET `/api/files` | none | `{"root":str,"current":str\|null,"files":[relative-path,...]}` | Not exercised live |
| `open_graph` | GET `/api/status` alive probe, POST `/api/open`, GET `/api/status?wait=1&timeout=30`, then `engine_status` | `{"path":absolute-path,"create":bool}` | open reply is ignored; subsequent status/state are consumed | Not exercised live |
| `node_library` | GET `/api/state` | none | optional `library` as above; authoring guide comes from the local Python package | Not exercised live |
| `wrangle_reference` | **no HTTP** | none | reference text comes from the local Python package | Not exercised live |

There are **15 method/path combinations** (GET+POST proxy count separately).
There are no screenshot, viewport-image, file-upload, save, library-write,
Blender-launch or Python-execution HTTP calls in this checkout's MCP tools.
The Python web editor has other endpoints; they are outside this contract.

If no engine answers, MCP `open_graph` can launch the **Python** engine. Therefore
start the Rust app first. MCP also falls back to the local Python registry for
`list_node_types`, and to local graph files for `list_graphs`, when disconnected.
These fallbacks are unchanged and must not be mistaken for a Rust connection.

## Shared JSON shapes

Snapshot:

```json
{"rev":3,"document_id":"opaque document identity","path":"absolute graph path",
 "graph":{"version":1,"nodes":{"box1":{"type":"box","params":{},"inputs":{},"pos":[0,0]}},"display":"box1","exposed":[]},
 "can_undo":true,"can_redo":false,"live_modifier":null}
```

Graph nodes retain arbitrary editor metadata including comments, layout and
network boxes. Rust also reports `dirty` on snapshots. Revisions are monotonic
cook-request tokens within a document, not a count of MCP calls.

Status (optional result fields are absent until available):

```json
{"rev":3,"cooked_rev":3,"busy":false,"frame":1,"fps":24,"ok":true,
 "points":8,"prims":6,"vertices":24,
 "cook":{"ms":0.3,"cooked":["box1"],"cached":[]},
 "proxy":{"level":1,"auto":false,"cooked":1,"link_tris":0}}
```

On cook failure: `ok:false,error:{node:str|null,message:str}`. The previous good
geometry counts may still be present. Before the first completed Rust cook,
`ok:null`. Rust adds `engine:"rust"` to status and
`engine:{engine:"rust",backend:"rust",pid,version:"0.1.0 (rust)",python_nodes:false,...}`
to state. The engine object is retained because unchanged MCP indexes its
`pid`, `version` and `python_nodes` fields. Consequently MCP's textual
`engine_status` visibly identifies Rust through the version string.

Node-type:

```json
{"name":"box","category":"rust","doc":"Rust core node",
 "inputs":[],
 "params":[{"name":"size","kind":"vec3","default":[1,1,1],"doc":"...",
            "min":0,"soft_max":10,"supported":true}],
 "actions":[]}
```

Actual inputs/defaults/supported flags come from the live Rust core registry.
Kinds, docs, ranges, choices and UI hints come from `param_meta.json` when present.
MCP requires each parameter's `name`, `kind`, `default`; it optionally consumes
`choices`, `min`, `max`, `soft_min`, `soft_max`, `ui`, `doc`. It consumes each node's
`name`, `inputs`, `params`, optional `category`, `doc`, `source`, `actions`.

Successful inspection:

```json
{"ok":true,"node":"box1","rev":3,"ms":0.1,"cooked":[],"cached":["box1"],
 "points":8,"prims":6,"vertices":24,"verts_per_prim":[4,4],
 "bounds":{"min":[-0.5,-0.5,-0.5],"max":[0.5,0.5,0.5],"size":[1,1,1],"center":[0,0,0]},
 "attributes":[{"name":"P","domain":"point","type":"float32","components":3,
                "min":[-0.5,-0.5,-0.5],"max":[0.5,0.5,0.5],"mean":[0,0,0],"first":[[-0.5,-0.5,-0.5]]}]}
```

`verts_per_prim` is omitted with no primitives; bounds are omitted with no points.
Ranges are per component. `detail=false` omits ranges/mean/first. Detail attributes
use `{name,domain:"detail",value}`. Failure is HTTP 200 with
`{ok:false,node,rev,error:{node,message}}`, including upstream failing-node IDs.
Python additionally includes `flow`, and sometimes `wrangle`/`blender` diagnostic
metadata; Rust currently omits those optional diagnostics. MCP does not index
them. Inspection runs on the existing worker session, retaining live external
inputs, timeline and caches; it does not construct a disconnected graph evaluator.

## Atomic edit vocabulary

Each batch is evaluated on a private JSON snapshot and validated before the UI
document is touched. An invalid operation returns HTTP 400
`{"error":"op N (kind): reason","index":N}` and changes neither graph nor history.
A valid incomplete graph is accepted: **cook failure does not roll back the edit**.
The reply reports the failing node and the edit remains undoable, exactly as MCP
expects. One changed batch is one History snapshot/undo step. The UI's ordinary
`commit_document` sends `Edit::Replace` to the existing CookWorker; the existing
Blender link then publishes the document, exposed interface and cook result.

| Op | Fields | Result object / behavior |
|---|---|---|
| `add` | `type`, `id?`, `params?`, `inputs?:{port:id|[ids]}`, `pos?:[x,y]`, `display?:true`, `comment?` | `{id}`; generated ID is type + first free positive integer |
| `insert` | `after`, `type`, `id?`, `params?`, `port?`, `pos?`, `comment?` | `{id,rewired}`; insert after source, redirect existing consumers and its display flag |
| `remove` | `id:str|[str]` | `{removed:[ids]}`; clean consumers, display/exposure references and owned object-input nodes |
| `rename` | `id`, `to` | `{id:new}`; update consumers, display and exposures, preserving node order |
| `connect` | `from`, `to`, `port?`, `index?` | `{port}`; default first free input then first input; single input replaces; multi input removes duplicate source then inserts at clamped index/end; cycles rejected |
| `disconnect` | `to`, `port?`, `from?` | `{}`; without source remove whole selected input(s), otherwise only matching source |
| `set` | `id`, `params:{name:value}` | `{params:{name:effective-value}}`; null removes override and returns default |
| `display` | `id` | `{}`; select display node, clear extra display metadata |
| `expose` | `id`, `param`, `name?`, `label?` | `{name}`; float/int/bool/vec3 sockets, avoid name collisions, update existing exposure |
| `unexpose` | `name` or `id` + `param` | `{}`; error if no match |
| `move` | `id` + `pos` or `positions:{id:[x,y]}` | `{}`; positions rounded to tenths |
| `meta` | `id`, `set:{key:value}` | `{}`; null deletes; type/params/inputs reserved |
| `replace` | `graph` | `{}`; validate whole replacement, preserve unknown metadata |
| `layout` | none | `{}`; arrange top-to-bottom; newly added nodes otherwise receive missing positions automatically |
| `loop` | `from?`, `to?`, `port?`, `iterations?` (default 3), `pos?` | `{id:end,begin,body,end}`; paired Begin/Transform/End, optional downstream connection |
| `box_add` | `around:[ids]` or `pos`+`size`, `id?`, `title?`, `color?` | `{id}`; network box |
| `box_set` | `id`, `title?`, `color?`, `pos?`, `size?`, `collapsed?`, `locked?`, `note?`, `text?` | `{}` |
| `box_remove` | `id:str|[str]` | `{}` |
| `paint_assign` | `id`, `indices?`, `value?`/`values?`/`erase?`, `clear?`, `default?` | Rust edits the Python-compatible `mhp1:` payload and returns `{edited:true,bytes:int}`. |

The MCP edit schema lists 18 op names; its prose also documents `paint_assign`,
which its Python HTTP client can send. Other editor-only operations in Python's
`_OPS` are not advertised by MCP and are not implemented here. Layout coordinates
and box-fitting algorithms need not be pixel-identical to Python's web editor.

## Open, frame, proxy, library and export behavior

`/api/open` validates a `.json` path inside the workspace and uses the same
`Document::open` / `App::open` path as the native File menu. `create=true` creates
the Python-compatible starter box/output graph only when the file is missing.
Bare new filenames are placed in `graphs/`; explicit subdirectories are honored.
Existing files are never overwritten by create. Open replaces document history,
and retains the HTTP and Blender listeners. Missing files and unsupported Python
node types produce clear errors (`node type 'T' is not in the Rust backend yet`).
The app rejects Open while a live Blender modifier document is active; disconnect
that live document first, as required by the native File menu too.

Frame accepts finite `abs(frame)<1e7`, and `0<fps<1000`; omitted fps retains the
current value. It sends the same `Edit::Time` as the Blender timeline, outside
document history. Proxy validates `0.001<=level<=1`, boolean auto and nonnegative
u32 link_tris and updates native viewport settings. Rust cooks full geometry:
`cooked=1`, `mode="viewport"` makes this distinction explicit. The existing UI
proxy/link cap is used; Python's cook-time resolution reduction is not emulated.

The node-library tool still returns its unchanged Python authoring guide. Rust
state marks `library.supported=false` and lists `.py`/system `.json` files with an
explicit unavailable error. It does **not** execute or hot-reload Python node
libraries or enable arbitrary Python nodes. Only Rust registry types can be added.

USD exports cook the selected Rust node, or the display and extra output nodes,
on the existing worker at full resolution. The already-cooked geometry is passed
to `tools/rust_usd_export.py`, which reuses `minihou.usd_io` for the exact OpenUSD
serialization and report (attributes, curves, loose points, instances, groups,
materials and optional graph). No Python graph evaluation or Python engine is
started. Requires Python/numpy/usd-core; set `MINIHOU_PYTHON` before starting the
Rust app if `python` is not the right interpreter. Missing support returns 501;
overwrite conflicts return 409. Export paths must remain in the workspace.
`usd_export`'s `save` button supports current-frame and animation-range writes,
including name/format/root/units/materials settings. Animation samples are streamed
one frame at a time and the worker timeline is restored even after errors.
Other unavailable buttons explicitly return 501.

## Verification

From `rust/`, with cargo on PATH and `CARGO_TARGET_DIR=.../rust/target_mcp`:

```powershell
cargo check -p minihou-app --tests
cargo test -p minihou-app --lib http_
```

The HTTP regression uses an ephemeral loopback port and a windowless editor pump
with the production HTTP transport, op validator, History and CookWorker. It
checks a multi-op real-socket edit, inspection, invalid-batch rollback, undo/redo,
retention of cook failures, and unchanged Python MCP calls against that Rust
service. Separate coverage checks live external inputs/time and invalid Host,
Origin and body-size requests. This is not native-window/Blender visual proof.
The native adapter's workspace path/library checks have separate binary tests
(`cargo test -p minihou-app --bin minihou-app http_ui`).
`python -m unittest tests.test_rust_mcp_support` verifies geometry reconstruction,
current-frame and streamed animation serialization calls, and writer error shapes
without requiring an installed OpenUSD package.

`python -m unittest tests.test_mcp` constructs its own Python WebServer/Session
and cannot be redirected to an external Rust app. It remains a Python regression
check. With the Rust app already running, use:

```powershell
python tools/mcp_smoke_rust.py
python tools/mcp_smoke_rust.py --edit
```

The smoke script starts nothing. Default is read-only; `--edit` opens a disposable
graph, checks mutation/rollback/undo/redo and returns to the initial graph. Close
or save unsaved work before using that option: Open resets document history.

Validation recorded for this change (2026-10-16): `cargo check -p minihou-app
--tests` passed; app library tests **66 passed** (including four HTTP/session
tests); native adapter tests **2 passed**; Blender-link protocol tests **14 passed**;
`tests.test_mcp` **15 passed**, plus serialization bridge tests **5 passed**.
The existing registration handshake still lists **14 tools**. Python modules
compile and the tracked-file whitespace check passes. Build/test logs are under
`rust/target_mcp/` (ignored).

No native app was launched or killed, and no release build was run. The live
smoke probe found no service on 8766, so the native window and actual Blender
display/undo behavior remain unverified. This environment lacks `usd-core`:
the real Rust-to-Python writer subprocess's structured 501 and timeline
restoration were tested; successful USD file authoring was not. Serialization
of current frames and animation samples was verified with the writer mocked.

Files changed for this work: `src/http_api.rs`, `src/http_ops.rs`,
`src/http_geometry.rs`, `src/http_ui.rs`, `src/http_tests.rs` under
`rust/crates/minihou-app`, targeted integration in that crate's `src/main.rs` and
`src/lib.rs`, `tools/rust_usd_export.py`, `tools/mcp_smoke_rust.py`,
`tests/test_rust_mcp_support.py`, `tools/setup_mcp.py`, `README.md`, `.gitignore`,
and this contract. No core node files or `blender_link.rs` were edited for this
work; concurrent helpers' changes are retained.

## Live check and fence sample (2026-10-17)

The debug native app was launched from this worktree and answered GET `/api/state`
with `engine.engine="rust"`, and GET `/api/nodes` with 183 types. The
`tests/test_mcp_live.py` live test passed through the Python `McpServer` tool
layer: `engine_status`, filtered `list_node_types`, `get_graph`, `edit_graph`,
`inspect_node`, `undo`/redo, and `export_usd`. The export created a real temporary
`.usda` file, which the test removed. An `edit_graph` batch added a box, connected
it to output, and set its size before inspection and undo. No requested tool
failed at runtime. The other seven tools in the table were not exercised live.

`tools/create_fence_mcp.py` created `rust/crates/minihou-app/src/test_data/fence.minihou`
with MCP `replace`, `add`, `connect`, and `display` operations. It cooked in the
live app with 488 points, 366 primitives, and no cook error. `garden_fence` uses
the new Rust `fence` node, whose input is an open curve and whose parameters are
bay width, height, rail height, slat width/gap, batten thickness, and post size.
The sample uses `sample_path` so it cooks headlessly. To use a Blender curve,
pick it for `blender_path` and connect `blender_path` to `garden_fence.path`.
The node outputs posts at bay boundaries, three rails per bay, lower slats and
crossed upper battens; its geometry detail includes `fence_bays`, `fence_posts`,
and `fence_slats` for validation.

The first debug build failed with OS error 112 (disk full). Cleaning only
`rust/target_fence` and building with debug symbols and incremental artifacts
disabled succeeded. The first targeted Rust test link then failed with LNK1180
(insufficient disk space). A second clean of only `rust/target_fence` allowed
the focused tests to pass: `minihou-core --lib fence` (1), `minihou-app --lib
fence` (1), and `minihou-app --lib mcp` (1). Blender's object picker, curve
feed, live modifier, and viewport appearance still require a Blender/native UI
session.

Rechecked on 2026-10-18 in this worktree: `tests.test_mcp_live` passed (1),
including MCP `edit_graph` add/connect/set and the documented state/node-type
fields. Targeted `minihou-core --lib fence`, `minihou-app --lib mcp`, and
`minihou-app --lib fence` each passed (1). The first rebuild attempts hit OS
error 112 twice as disk space fluctuated; `cargo clean` was confined to
`rust/target_fence`, and the subsequent debug build passed. These were build
environment failures, not MCP/API failures. No additional tool failure was
observed; the table's seven unexercised tools remain unverified live.
