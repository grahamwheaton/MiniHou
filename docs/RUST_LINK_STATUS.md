# Rust / Blender link status

Updated 2026-10-16. This describes the current checkout, including the add-on changes here. The Desktop backup was not changed.

## Dynamic modifier inputs (add-on 0.2.0)

The Rust `modifier_input` node stores ordered `modifier_inputs` definitions (`id`, `name`, `kind`, `default`, optional `min`, `max`, `subtype`, or menu `options`). The native params panel adds them. `link_interface` advertises every definition; `instance_cook` applies per-placement values to `input_values` before graph evaluation. A node parameter can bind to `{"modifier_input":{"node":"input","id":"input_1"}}`; the Rust graph resolves that value during cook, so existing numeric/string node parameters consume it without a new scalar-port evaluator. The add-on maps stable IDs to Blender socket identifiers, preserving modifier values and GN wires through rename/reorder. Removed inputs are removed cleanly. Independent instances keep separate values.

The 0.2.0 bridge lists float, int, bool, vector (translation/xyz/euler/direction), color, rotation, matrix, string, menu, object, collection, material, image, texture, geometry, bundle and closure socket types. `bpy.app.version` and available `bpy.types` guard newer sockets. Object and collection references cross as names; evaluated object meshes travel through `instance_sources`, with names/transforms in `references`. Wired geometry inputs also travel as mesh sources. Existing `object_input` nodes can read an `input:<modifier node>:<stable id>` binding. Material/image/texture references are available as names to string-accepting Rust nodes. Linked bundle/closure values report an unsupported-data error. Computed links for ID/matrix/rotation/menu sockets have not been sampled; unsupported computed links report an error instead of being guessed.

Mocked bpy interface tests and the headless Rust link test cover float, int, bool, vector, color, string and object round trips, independent placement values, and stable socket identifiers on rename/reorder. Actual Blender socket construction, modifier panel behavior, evaluated object/geometry sampling, reference metadata and UI interaction remain unverified without Blender.

## Implemented contracts

| Add-on path | Rust behavior | Evidence |
| --- | --- | --- |
| `runtime._apply_interface`, proxy sockets | The params panel emits `Edit::Exposed`; history saves the document's top-level `exposed` array. `link_interface` resolves numeric/bool/vec3 socket metadata, current/default values, labels and a structure token that does not change for value-only edits. `document_changed` broadcasts changes without requiring a geometry change. | egui pointer-click test for the actual Expose control; socket test for interface creation/removal, set_param by exposed name, geometry update, undo/redo and unchanged source file. |
| `LinkClient.set_param`, native history | Exposed names and direct node/param addressing use the app's normal validation and undo history. Parameter changes reach the cook worker and return in interface/geo messages. | `test_exposure_interface_roundtrip_and_native_history`; existing library validation/history tests. |
| `InstanceClient` / `modifiers` / staged `geonodes` | `hello mode=instances` receives only welcome and solicited replies, with request/instance correlation. All six RPCs work: describe, cook, sources, edit, sync, remove. Each connection owns its instances, sources and bounded session caches. Each cook starts from supplied graph defaults plus the complete override set; removed overrides do not leak. Render requests respect the graph's render target. Invalid requests return `instance_error` and leave the connection usable. | Instance integration tests using the actual shipped add-on client, including two connections with the same ID, chained geometry, materials, attributes, defaults, render target and recovery. |
| `modifier_input`, `object_input` | Instance cooks feed the preceding-stack geometry into the existing core `modifier_input` node. Object-input bindings use a separate per-instance namespace, with replacement/removal of sources. `blender_object` is rejected recursively in modifier graphs to prevent feedback; the main output must depend on a top-level modifier_input. | Isolation, source removal/recovery and feedback-error tests. No process-global input store is used. |
| Live Edit Network | `instance_edit` selects a separate native document with transient input/source/time feeds. The original document, path, save point, history and timeline are retained; remove/disconnect/restart restores them. Per-live-document history survives selecting another instance. `instance_sync` merges only explicitly changed Blender values and preserves native edits, including edits made between a feed request and UI processing. Float32-equivalent values do not cause feedback. | Live integration tests covering native edits, Blender edits, source/input updates, instance switching and disconnect restoration of original history/time. |
| Native editor routing | Live replies identify `editor: native`. Both modifier and GN callers propagate this to `NetworkEditor`, which acknowledges the already-bound native document without requiring a browser URL. The app requests focus on activation. Python server/browser routing is retained. File Open is blocked while a live instance is selected, preventing another document from accidentally becoming that modifier. | Actual pure add-on router tested; existing Python HTTP/browser routing regressions pass. Window focus is unverified. |
| TCP / capabilities | Welcome advertises `instances_v1`, `live_modifier_v1`, `object_inputs_v1`, `scene_objects_v1`, `image_exchange_v1`. It does **not** advertise `shm_v1`. Both shipped clients ask for shared memory and successfully receive ordinary MHL1 TCP frames. `geo_ack` remains harmless. | All socket tests use the shipped clients, without shared memory. |
| Diff / sequence / resync | Each subscriber has independent main and named-output diff state. Sequence numbers belong to the listener rather than a document worker, so File Open cannot rewind them. Resync clears all geometry/extra/scene baselines. Removed attributes disappear through the manifest; empty `prim_closed` clears curve flags. Point-count changes also request a topology rebuild. Failed cooks preserve the last good geometry and its sequence for late subscribers/resync, while errors have their own newer sequence. | Two-client/conflation/resync integration tests, malformed input and recovery, curve-to-polygon and attribute removal, Rust worker-counter-reset and loose-point-count tests. |
| Timings / detail | Geo replies carry `cook.ms`, cooked/cached node lists, `node_ms`, frame/fps, `t_sent`, counts and JSON detail. Material slot names survive input and result transport. Instance result replies include a complete mesh plus interface and optional scene payloads. | Integration assertions and protocol unit tests. |
| Named outputs | `outputs.names` and independent `geo.output` messages follow the graph's unique trimmed output names, with per-name diffs/full resync and removal announcements. | Named-output integration and Rust socket tests. |
| Blender object/image watches | The server broadcasts deduplicated `watch_objects` lists, accepts geometry/missing and image-data/missing replies, invalidates malformed replacement inputs, recooks and recovers. Object matrices/info/materials and image metadata are decoded. | Real TCP object, curve, malformed input, image and missing/recovery cases; matrix/image protocol unit tests. |
| `scene_nodes`, `scene_objects`, `image_out` | Scene inventory and content-keyed manifests are sent with binary arrays. Instances, pieces, curves, points and image modes are covered. Failed side sinks do not hide a valid main mesh. Instance scene arrays use `scene:<node>:<key>` and errors are correlated by sink. | All four object modes and image manifests, named mesh output, resync and removal tested using the add-on's mirror/scene-plan contracts. |
| Staged GN scene payloads | Shared `instance_payload` separates side-sink arrays from mesh arrays. Both stack modifiers and staged GN route manifests to the existing scene queue. GN owners remain retained and removed placements queue empty inventories. This fixes the previous GN path feeding namespaced arrays into mesh attribute writing. | Payload split/mirror assertions and Python syntax/regressions. Actual GN execution/scene realization still needs Blender. |

## Test host and reproduction

`rust/crates/minihou-app/examples/link_selftest.rs` reuses the production `blender_link.rs`, core evaluator, Document and History. It opens no window or GPU. It accepts `GRAPH.json ADDRESS SECONDS [CONTROL.json]`, binds an ephemeral port with `127.0.0.1:0`, prints `LINK_READY <actual address>`, and exits on its deadline or a stop control. The optional command file simulates native exposure/parameter/history edits for integration tests. It is an example executable, not a new desktop-app CLI mode.

`tests/test_rust_link.py` loads the actual bpy-free add-on client/protocol/state/instance-client/router/scene-plan files in an isolated package, avoiding the bpy-dependent add-on entry point. Each test starts its own debug Cargo example with temporary graph/control files and a temporary listener. It never finds, launches or kills a running desktop app. It uses no Python engine/server as a substitute for Rust.

From the repository root:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:CARGO_TARGET_DIR = Join-Path (Get-Location) 'rust\target_inst'
python -m unittest discover -s tests -p test_rust_link.py -v
python -m unittest tests.test_protocol tests.test_network_editor tests.test_scene_plan tests.test_scene_objects tests.test_images tests.test_geonodes_engine tests.test_modifier_instances -v
Set-Location rust
cargo test -p minihou-app --lib --example link_selftest
cargo test -p minihou-app --bin minihou-app ui::params::tests
cargo check -p minihou-app --all-targets
```

The targeted Rust test commands execute test harnesses; they do not run the desktop executable. Do not replace them with an unrestricted `cargo test -p minihou-app` when the desktop executable must not be invoked: existing `tests/cli.rs` explicitly runs it.

Results: eight Rust-server socket integration tests and 106 existing Python regressions pass. The final targeted Rust run passed 66 library tests, 14 example/server tests and all six params-panel tests (including the egui Expose-button test). `cargo check --all-targets` and syntax checks on all changed Python files pass. Logs are under `rust/target_inst/link-*.log` (local generated evidence, not a release artifact).

Validation caveat: an initial broad Cargo test run also ran the pre-existing CLI tests, which invoked `minihou-app.exe` with `--selftest` despite the requested restriction. No window was opened. Subsequent/final runs use only the targeted commands above. No release build or Blender launch was performed.

## Remaining gaps / unverified behavior

- **No live Blender acceptance:** actual socket creation/removal, modifier-stack prefix evaluation, wired/nested GN evaluation, Blender material/data-block updates, scene-object/image realization, undo UI, depsgraph feedback behavior, and native window focus have not been smoke-tested in Blender. The egui test proves control dispatch and saved exposure state, not Blender socket creation. No claim of host-app acceptance is made.
- **Shared memory:** deliberately unimplemented and unadvertised. TCP fallback is tested with both add-on clients. Large-geometry throughput and interactive latency are unbenchmarked.
- **Rust graph support:** protocol parity does not add Python-only nodes, Python node-library execution, arbitrary attribute dtypes, or unsupported core node parameters. The add-on mesh readers already convert generic booleans to int32; core geometry attributes support float32/int32, with byte/bool topology flags. Unsupported cases return explicit errors. In particular, the optional non-empty `image_out.file` PNG side effect remains unsupported by the Rust core; network image exchange works.
- **Native presentation:** native live editing uses temporary independent JSON documents rather than the Python HTTP/browser editor. The TCP welcome has no browser `editor_url`. The generic Start Server/Open Browser controls still belong to the existing Python server-control workflow; this work enables the live modifier/GN native route. Native preview currently shows core instance-point geometry rather than Python's source-mesh realization preview. These presentation differences do not change geometry sent back to Blender.
- **Proxy policy:** Rust sends its cooked geometry; Python's adaptive cook-proxy policy is not ported by this change. The native viewport's display LOD remains a separate feature. No shared-memory or adaptive-proxy capability is claimed.
- **Side-output diagnostics:** the current `CookResponse` exposes a single aggregate error string for proxy-side extra/sink failures, so the last such error is what the TCP proxy error reports. Valid main/other successful outputs still flow. Instance replies have separate per-node `scene_errors`.

## Live Blender smoke test to perform later

For the 0.2.0 input feature:

1. Install add-on 0.2.0 in Blender 5.x, connect to the Rust link, select `modifier_input`, and add float, int, bool, vector, color, string, object, plus a second geometry input in the native params panel. Confirm each appears on the Plumbing node and applicable controls on the modifier.
2. Link float/vector inputs to a Box/Transform parameter in MiniHou; set different values on two placements and cook. Confirm different mesh results and the correct values in the native editor. Pick an object, bind `object_input.binding` to `input:<modifier node>:<stable id>`, and confirm its evaluated mesh arrives.
3. Rename and reorder inputs, then remove one. Confirm retained sockets keep their Blender identifiers, links, modifier values and animation, while the removed socket/wires disappear cleanly; save/reopen the `.blend` and repeat.
4. Try color, string, rotation, matrix, menu, material, image, texture, geometry, bundle and closure on Blender 5.x. Check supported literal/reference values round-trip, wired geometry cooks, and unsupported linked/computed combinations display an error while the last cache remains visible.

Previous broader link smoke plan:

1. Install the current checkout add-on, connect to the Rust app, select a Box size parameter and click Expose. Confirm the document's `exposed` list, the proxy modifier's new vector socket, and socket removal when toggled off.
2. Change that Blender socket and confirm native geometry/value updates; undo/redo in the app and check Blender follows without repeated feedback cooks.
3. Add two instances of `examples/modifiers/transform.json` after different native modifiers. Change their controls independently, chain them, and verify material slots and point/corner attributes.
4. Use Edit Network on each stack/GN occurrence. Confirm the correct native document/input is selected, edits travel both ways, and disconnect/remove/restart restores the original document and undo history.
5. Exercise object/image watches and every scene_objects mode, including source deletion/reappearance, renamed outputs, removed scene sinks, and image painting. Confirm real Blender data-block updates and cleanup/parking behavior.
6. Repeat a wired/nested GN chain with scene sinks and duplicated placements. Confirm there are no scene-prefixed mesh attributes and each occurrence owns its scene output.
