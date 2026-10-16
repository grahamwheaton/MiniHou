# Native viewport textures

Design only, inspected 2026-10-16 at `6effa42`; no implementation or host-app validation in this task.
Goal: Blender-linked meshes display base-colour/albedo images with correct UV seams and face slots; extend later to normal, roughness, metallic, emission and alpha.

## Current checkout: evidence and gaps

- `rust/crates/minihou-app/src/viewport.rs`: `Scene::new` retains positions, point normals and point-indexed triangles/edges; it drops UVs, `Cd`, material slots and `preview::Layout::tri_prim`.
- `Gpu::upload` uploads geometry; `shader.wgsl` accepts position/normal only and uses one base colour with studio lights, or a generated clay MatCap texture. MatCap UVs derive from view normals, not mesh UVs.
- The egui target is a registered wgpu texture; the shader manually encodes colour for the non-sRGB editor target. `display_modes.rs` supplies 12 Houdini-style bounds, wire, hidden-line, flat/smooth and MatCap modes; X-ray/selection/ghost/outline passes are separate concerns.
- `cluster_lod.rs`, core `lod.rs` and `proxy.rs` change triangle sets; current LOD clusters carry point-indexed triangles without UV/material provenance. Textures cannot safely be bolted onto that stream.
- `rust/crates/minihou-view/src/{main.rs,shader.wgsl}` is a separate viewer with untextured workbench shading, not the native editor implementation target.
- `minihou/preview.py` sends point `Cd`; `minihou/web/static/preview.js` is plain WebGL2 (no three.js), shading colours with a headlight, without material image sampling.
- [MATERIALS.md](MATERIALS.md) describes name/slot transport and Python/USD behavior, not native texture support. [VIEWPORT-PARITY.md](VIEWPORT-PARITY.md) is the older browser roadmap; native source is authoritative for current display modes.
- `blender_addon/minihou_link/mesh_reader.py` exports evaluated geometry: render UV layer as `vertex/uv`, other UV layers by name, face `prim/material_index`, slot names in header `materials`; no material node graphs or image bindings.
- `image_exchange.py` separately exports bottom-up RGBA float32 pixels plus size, `is_float`, colour-space and source metadata for image nodes, capped at 16,000,000 pixels. `runtime.py` watches presence/`is_dirty`; this is not a material asset cache.
- `link_protocol.rs`: MHL1 prefix is magic + little-endian u32 JSON length + u64 blob length (16 bytes); JSON and binary arrays use 8-byte alignment and typed shape/offset descriptors. Nominal limits: 64 MiB header, 8 GiB blob; `blender_link.rs` actually accepts 1 MiB hello and 512 MiB subsequent frames.
- [RUST_LINK_STATUS.md](RUST_LINK_STATUS.md): `image_exchange_v1` works, `shm_v1` is not advertised; shipped clients fall back to TCP. Add-on `shm.py` already has aligned buffers and a 64 KiB threshold, but native shared memory is future work.
- Core `geometry.rs` stores numeric point/vertex/prim attributes in shared arrays, JSON detail separately; vertex means polygon corner. `modify.rs` transform clones geometry and changes P/N, preserving UVs and slots.
- Core `nodes/merge.rs` concatenates attributes with zero fill, but explicitly rejects multi-input `materials`, `usd_materials` and `instance_objects` detail: material reconciliation must be implemented, not assumed from the Python docs.
- `nodes/subdivide.rs` interpolates float corner attributes within each face and gathers parent primitive attributes; `nodes/boolean.rs` blends source-corner attributes and remaps material names. Other topology nodes require individual provenance tests, not a blanket preservation claim.
- [RUST_USD_STATUS.md](RUST_USD_STATUS.md), core `usd_io.rs`/`usd_io/reader.rs`: pinned pure-Rust `openusd`/`openusd-schemas` revision `64bae2609d78565adbc8e8df8c575c7526815a00`; `st` maps to `uv`, bindings to slots. PreviewSurface values/referenced networks survive, but textures are not evaluated, copied or bundled, including USDZ output.

## Data model and ownership

- Retain `detail.materials` names and int32 `prim.material_index` for round-trip compatibility; add versioned `detail.material_bindings`, a parallel slot-to-immutable-material-ID array (null = existing neutral fallback).
- Store small material descriptions in a document/session registry; cooked geometry contains IDs, never image bytes. Identity includes source namespace (connection/document/library), not just Blender's display name; canonical descriptor hashes identify revisions.
- An application asset service owns immutable images keyed by SHA-256 of canonical descriptor + payload (dimensions, pixel encoding, colour interpretation and row origin included). Transfer only missing hashes; names/paths are provenance, never identity.
- Keep original transfer payload and decoded pixels outside Geometry/cook caches; GPU resources are device-local derivatives. Material revisions trigger scene rebinding/redraw without forcing geometry upload; binding/UV changes invalidate the relevant scene conversion.
- Persist descriptors and referenced assets in a workspace asset store for saved documents; transient link sessions can remain memory-only. Reload requests missing hashes or reports missing assets; it must not silently substitute a same-named image.
- Example material JSON (factors are linear, texture sample multiplies base factor):
```json
{"version":1,"name":"Wood","base_color_factor":[1,1,1,1],"textures":{"base_color":{"image_id":"sha256:…","uv_set":"uv","sampler":{"min":"linear_mipmap_linear","mag":"linear","wrap_u":"repeat","wrap_v":"repeat"},"color_space":"srgb"}},"alpha_mode":"OPAQUE","alpha_cutoff":0.5,"double_sided":true}
```
- Reserve texture roles `normal`, `roughness`, `metallic`, `emission`, `alpha`, channel selectors and normal scale; scalar maps are linear data, emission/base colour have explicit colour interpretation. Arbitrary Blender shader evaluation is out of scope.

## Link additions and Blender extraction

- Negotiate `viewport_materials_v1` and `image_assets_v1`; retain existing message meanings and untextured behavior with older peers. Apply to object watches, modifier inputs and `instance_sources`, scoped by session/instance/request.
- Add `material_manifest` (revision, descriptors, bindings, image descriptors), `image_request` (missing hashes), `image_begin`, `image_chunk`, `image_end`, `image_ack` and `image_error`; geometry references a manifest revision so out-of-order arrival cannot bind the wrong slots.
- Transfer chunks as MHL1 `|u1` arrays, initially at most 1 MiB each; include transfer ID, image hash, byte offset and total length. Verify complete coverage and final hash before atomic publication; bound queues and allow cancellation/retry/resync.
- Negotiate conservative limits independently of the existing 8 GiB frame ceiling: initially 16 Mpx/image, 256 MiB decoded/image, 512 MiB total in-flight; reject overflow, overlap, inconsistent dimensions and decode expansion before allocation. Expose downsample/limit errors rather than freezing the UI.
- Prioritize control/geometry traffic between chunks; receive/decode/hash off the UI thread, upload within a per-frame budget. Reconnect negotiates retained hashes; acknowledgements mean verified availability, not permanent residency.
- Default to byte transfer for external, packed and generated images. A local file-path hint is optional only with explicit shared-path capability, canonical allowed roots and hash verification; otherwise request bytes. Never assume Blender's `//` path exists on the receiver.
- Extract on Blender's main thread from the evaluated object's slots: walk the active Material Output to Principled BSDF, resolving reroutes and bounded group links; initially accept direct Image Texture -> Base Color plus UV Map/default render UV and simple supported Mapping transforms.
- Record the render-layer-name -> `uv` alias explicitly. Unsupported mixes, procedural coordinates/nodes, movies/sequences or ambiguous outputs get a visible diagnostic and constant fallback; do not silently guess or bake.
- New `material_export.py` collects descriptors/dependencies; reuse `image_exchange.py` for safe pixel reads. Read packed bytes when suitable, or snapshot `image.pixels` for generated/painted content; never pack, save or modify the user's Blender datablocks as a side effect.
- Initially accept verified PNG/JPEG bytes for ordinary images; use Blender-decoded pixels for other formats, packed sources and edits. Preserve 16-bit/EXR precision via normalized linear RGBA float32 transfer and RGBA16Float GPU storage; encoded 16-bit PNG decoding must retain precision, never silently reduce to 8-bit.
- Normalize colour through Blender's configured colour-management path to a declared linear working space when source encoding is ambiguous; do not infer solely from `is_float`. Test byte/file and pixel paths separately; non-colour data bypasses colour transforms, and row orientation is explicit.
- Detect material/depsgraph changes and image `is_dirty`, filepath + mtime/size, packed-data/source changes; debounce and hash snapshots. Dirty can stay true: compare hashes, not a dirty flag edge. Provide explicit Refresh for missed external changes; do not clear Blender's dirty state.
- Reserve UDIM tile-number -> image-ID maps; MVP reports tiled images unsupported rather than stretching tile 1001 across the mesh. Tile loading, sequence frames and baking are later extensions.
- Shared-memory follow-up may transport identical immutable chunk payloads through negotiated slots with ownership/ack lifetime rules; TCP remains mandatory and identical semantically. Texture delivery does not depend on implementing SHM.

## Engine and renderer changes

- Keep UVs as float2/float3 corner attributes (sample XY); permit point UV fallback only when no corner UV exists. Preserve named UV sets and material bindings through link decoding, detail serialization, copying and graph caches.
- Factor material-table reconciliation out of boolean handling: merge by material identity, preserve legacy names/empty slots/USD source references and remap face indices; never average integer material indices. Include new detail in validation and cache identity.
- Topology contract: retained faces gather slots/corners; splits interpolate UVs within their source face; subdivisions inherit parent material; new caps/faces without a source use a neutral binding and invalid-UV marker until a UV node or explicit material assignment supplies them.
- Nodes must report dropped UV sets or unsupported provenance. Audit merge/copy/delete/subdivide/boolean/extrude/bevel/fracture first; tests should distinguish inherited faces from newly generated interiors.
- Extend core `preview.rs` triangulation with triangle-corner and primitive provenance. Build seam-split render vertices from point + corner UVs + normal; retain original point/primitive IDs for selection, overlays and ownership. Honour corner N before point/computed normals.
- Partition draws by material, alpha mode and existing selected/visible/ghost ownership ranges. Keep picking/outline IDs stable across reordering; points, curves, bounds and wire overlays retain their existing geometry paths.
- Use per-material group(1) bind groups (uniform factors + image view + sampler), leaving group(0) camera/MatCap bindings intact. Cache samplers/bind groups and use white/default textures for absent slots.
- Avoid texture arrays initially (mixed sizes/formats require resizing/bucketing) and bindless (optional adapter features); revisit only after profiling material draw-call overhead on the checkout's wgpu 27 DX12/Vulkan paths.
- Decode base-colour sRGB exactly once using sRGB GPU views; shade in linear space and encode exactly once for the egui target. Replace the current approximate output gamma path coherently; float linear and data images use non-sRGB views.
- Generate the full mip chain on upload, filtering colour in linear space; honour repeat/clamp/mirror and nearest/linear settings, with adapter-limited anisotropy. Flip Blender's bottom-up rows once at ingestion, with an asymmetric UV fixture proving orientation.
- MVP is opaque base colour under existing studio lighting in Flat/Smooth and their wire-shaded modes. MatCap retains its own captured colour; bounds/wire/hidden-line remain diagnostic modes. Missing images/UVs show factor colour and an inspector warning.
- Preserve material/UV provenance when sampling proxy triangles. Disable clustered simplification for textured surfaces initially (show the reason and use provenance-preserving triangle sampling); later make collapses seam/material-boundary aware before restoring textured LOD.
- Later MASK discards consistently in surface/depth/selection-mask passes; BLEND draws after opaque, depth-tested without depth writes, back-to-front, with intersecting-surface limitations documented. Compose with X-ray/ghost alpha explicitly; don't equate X-ray with material transparency.
- Later normal maps require seam-aware tangents/handedness; roughness/metallic require a defined PBR lighting model, emission adds linear radiance. Reserve schema now, but do not advertise these as rendered in MVP.
- Start with configurable 512 MiB GPU and 1 GiB CPU asset budgets, counting mip levels/staging/decoded copies; LRU-evict unreferenced resources, pin visible assets within budget and downsample visible over-budget assets with a status indication. Re-request evicted missing payloads safely.

## UI and validation

- Add a persisted **Materials/Textures** toggle to the existing shading menu (also reachable from Overlays); independent of the master overlay switch. Explain inactivity in MatCap/diagnostic modes; default on for supported shaded modes.
- Texture inspector lists slots, image thumbnails/IDs, UV set, dimensions, colour space, sampler, resident bytes and pending/missing/unsupported state; Refresh and cache-budget controls use the same asset service.
- Extend `display_render_tests.rs`/`geometry_render_tests.rs` and `Target::new_headless`: render asymmetric checker UV seams, two slots, factor multiplication, sRGB swatches, wrap/mips, missing assets, proxy selection and unchanged MatCap; compare tolerant pixel regions/reference images, not exact cross-GPU bytes.
- Existing headless tests can skip when no adapter exists: require a GPU CI job to treat that as unavailable validation, never a visual pass. Add CPU provenance tests plus shipped-client socket tests in `tests/test_rust_link.py` for dedup, chunks, limits, reconnect and independent instances.
- Blender smoke acceptance: a two-material seam mesh with external, packed and generated images; paint/update and reconnect without duplicate bytes, transform/merge/subdivide, saved-document reload, then compare native viewport against the source UV layout. Headless tests do not prove Blender UI behavior.

## Staged implementation (S/M/L; proposed files, not changes in this task)

1. **M — Contract and engine provenance.** Core `geometry.rs`, `preview.rs`, `nodes/{merge,boolean,materials_node}.rs`, new material helpers/tests; document schema and new-face rules. Gate: seam/slot reconciliation tests. Agree the schema before branches below diverge.
2. **L — Asset service and transport.** App new `image_assets.rs`/`materials.rs`, `link_protocol.rs`, `blender_link.rs`, `link_instances.rs`; add-on `protocol.py`, `client.py`, `instance_client.py`; socket tests. Gate: bounded, hash-verified TCP dedup/resync. Can run alongside 3 and renderer scaffolding in 4 after 1's contract.
3. **M — Blender exporter.** New `material_export.py`, `mesh_reader.py`, `image_exchange.py`, `runtime.py`, modifier/GN source hooks and tests. Gate: supported node paths, packed/generated/HDR snapshots and independent image updates. Parallel with 2 using manifest fixtures; integration waits for 2.
4. **L — Native opaque rendering.** App `viewport.rs`, `shader.wgsl`, `main.rs`, `proxy.rs`, `cluster_lod.rs`, core preview provenance integration; mip/upload cache and headless fixtures. Gate: textured seam mesh, multiple slots and stable selection. Uses 1; fixture-driven work parallels 2/3, end-to-end acceptance waits for both.
5. **M — UI and MVP acceptance.** App `ui/{viewport_ui,prefs,editor_state}.rs`, new inspector, render/link tests and status docs. Gate: toggle, budgets, reload/reconnect and real Blender smoke test; UI scaffold can parallel 4, release validation follows 2–4.
6. **L — Later channels/interchange.** Material exporter/service/shader plus tangent helpers and core `usd_io.rs`/`usd_io/reader.rs`: add normal/roughness/metallic/emission, MASK/BLEND, then supported UsdUVTexture extraction/export and explicit asset packaging. Separate acceptance per feature; USD adapter work can parallel renderer channels after the schema stabilizes. UDIM, SHM and textured clustered LOD remain separately scoped follow-ups.
