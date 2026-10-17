# Embedded MiniHou networks (add-on 0.3.0, link capability `graph_embed_v1`)

A MiniHou modifier used to reference its network only through a file path, so a `.blend` sent on
its own lost the network. From add-on 0.3.0 the network can live **inside the .blend**, while
separate `.minihou` files remain available for sharing and central storage.

## Storage (file layout)

| Where | What |
| --- | --- |
| Text datablock `MiniHou: <object> <modifier>.minihou` | The graph JSON, one per network. `use_fake_user=True`, so it survives Purge and Save. Custom properties on the text: `minihou_owner` (network id) and `minihou_embedded_graph` (layout version 1). Text is deterministic (`json.dumps(indent=1, ensure_ascii=False)` + newline, key order preserved because node order is meaningful). |
| Node group of the modifier / GN placement (ID properties) | `minihou_embed_mode` (`EMBEDDED`/`LINKED`), `minihou_embed_text`, `minihou_embed_schema` (1), `minihou_embed_addon` (add-on version that wrote it), `minihou_embed_hash` (sha256 of the text's UTF-8 bytes), `minihou_embed_size`, `minihou_embed_synced` (Unix time of the last write), `minihou_linked_file`, `minihou_embed_paths` (original absolute paths), `minihou_embed_missing`, `minihou_embed_state` (`conflict` / `unreadable`, session only), `minihou_embed_error`, `minihou_freeze`. |
| `minihou_graph` (existing) | The runtime copy the engine receives: same graph with file paths resolved for this machine. |

The graph's own schema is its top-level `version` (currently 1, `minihou_core::graph::GRAPH_VERSION`).

## Modes

* **Embedded** (default for every new stack modifier and GN placement): the `.blend` holds the
  authoritative copy. Every edit that arrives over the link (`instance_synced`) is written to the
  text after 0.75 s of quiet (timer), and all pending/changed networks are written again in a
  `save_pre` handler. `load_post` reads the text back into the runtime copy (adopting hand edits
  made in Blender's Text Editor).
* **Linked File** (every network made before 0.3.0, and after *Switch to Linked File*): the file is
  authoritative and no text is used. Existing path modifiers are **not** migrated automatically;
  they stay linked until the user clicks *Embed from File* or *Switch to Embedded*.

Switching Embedded -> Linked requires an existing file that matches the embedded copy (otherwise the
panel asks for *Make file match embedded* or *Use file* first), then deletes the text.

## UI (Properties > Modifiers > MiniHou, per modifier)

* Status line: mode, size, `#` + first 8 hex digits of the hash; warning above 2 MB (attrib_paint
  payloads and sketch points grow); errors, missing files, absolute paths outside the blend folder,
  and "Waiting for your answer in the MiniHou window" while a conflict is open.
* **Open in MiniHou** (was *Edit Network*): pushes the embedded graph, then the existing live edit.
* **Embed from File**, **Extract to .minihou File...** (file browser), **Switch to Linked File /
  Embedded**.
* When an embedded copy and its linked file differ: which one is newer (file mtime vs.
  `minihou_embed_synced`) and **Use embedded / Use file / Make file match embedded**.
* **Freeze result: On/Off** with the saved cooked-mesh size.
* *Reload Graph* re-reads the linked file when it exists (and re-embeds it), else the embedded copy.

## Link messages (`graph_embed_v1`)

Advertised in the welcome capabilities; MHL1 framing, instance-mode connections. Older apps do not
advertise it: the add-on then skips the push and uses the ordinary live edit exactly as 0.2.x.
Older add-ons never send these messages.

| Message | Direction | Fields |
| --- | --- | --- |
| `graph_push` | Blender -> app | `request_id`, `instance_id` (network id), `schema` (1), `name`, `label` (`<object> / <modifier>`), `json` (exact graph text), `hash` (sha256 of its UTF-8 bytes), `size` (bytes) |
| `graph_ack` | app -> Blender | `status`: `unchanged`, `accepted`, `app_newer` (+ `json`/`hash`/`size` of the app's copy) or `conflict`; `schema`, `graph_schema`, `received` |
| `graph_pull` | Blender -> app | `request_id`, `instance_id` |
| `graph_state` | app -> Blender | `status`: `ok` (+ graph), `none`, `pending`, `resolved` (+ `resolution`: `use_embedded`/`keep_mine`/`open_as_new`, + the app's graph when a bound document exists) |
| `instance_synced` (existing) | app -> Blender | now also `hash` and `embed_pending` |

Limits and validation: graph text at most 32 MiB on both sides (nothing is truncated; larger
networks must use Linked File mode); `size` must match, `hash` must match the bytes; an envelope
`schema` above 1 is refused with an "update MiniHou" message; graphs are migrated (unversioned ->
version 1) and a newer graph `version` is refused. The add-on verifies the hash of every graph it
adopts from the app. Every RPC still has the add-on's 200 ms main-thread budget, so very large
networks may need a retry on a slow machine.

## App behaviour

* Receiving a graph binds a document to the Blender network id. The title is
  `MiniHou - Blender: <object>/<modifier>`. Edits sync back to Blender through the existing live
  path, and the add-on writes them into the text ("Save to Blender" is implicit).
* **File > Save As** on a bound document exports a separate `.minihou` copy and keeps the binding
  and the save point unchanged (it previously re-pointed the live document to the new file).
  Open/Save As dialogs accept `.minihou` and `.json`.
* Decision (pure function `graph_embed::decide`, unit tested): no bound document -> adopt (but if
  the app's own document has unsaved edits -> ask); identical -> nothing; app copy unedited since the
  last agreement -> adopt Blender's; Blender's copy unchanged since then -> send the app's back;
  both changed -> ask.
* The egui modal offers **Use embedded graph** (replaces the bound document through undo history),
  **Keep my version / Keep my open file**, and **Open embedded as new document** (for a bound
  document it first saves the app's version to a `.minihou` file you choose; for an unrelated open
  document, the document is kept in memory and restored when the Blender edit ends, as live editing
  always did). Unsaved work is never overwritten silently. The add-on polls the answer with
  `graph_pull`; while it is pending the embedded text is never overwritten by the app's copy.

## External files

On write, file parameters (`file` and `*_file` string params: image_file, image_out, usd_import,
usd_export, lsystem rule/image files, ...) are stored relative to the `.blend` (`//tex/a.png`)
when possible, with the original absolute path kept in `minihou_embed_paths`. On load they are
resolved against the current `.blend` location (falling back to the stored original), and missing
files are listed in the panel. *Extract* writes absolute paths so the file works anywhere.
Paths on another drive or outside the blend folder stay absolute and are flagged in the panel.

## Freeze result

The stack modifier's private cache mesh (`MiniHou Cache <id>`) already holds the last cooked output
and the plumbing reads it, so a `.blend` opens with correct geometry on a machine without MiniHou.
*Freeze result* (default **On**) keeps that behaviour and shows its cost (points and approximate
bytes). **Off**: the cache mesh is swapped for an empty one only while the file is written
(`save_pre`/`save_post`), so the file is smaller; on load the modifier passes its input geometry
through until the live link cooks again. The live link always re-cooks and replaces the cache.
GN placements keep their existing cache behaviour (not toggled yet).

## Duplicates, deletion, repair

`embed.repair()` runs in every pump after identity repair: a duplicated object/modifier (which gets
a new network id) receives its own copy of the text; a missing text is rebuilt from the runtime copy
(with a panel note); texts whose owner network no longer exists are removed, and a deleted
modifier's text is removed together with its node group. User texts are never touched (only texts
carrying `minihou_embedded_graph`). Repeated syncs keep datablock user counts stable (tested).

## Security

Embedded python node code travels with the `.blend` like any other parameter, but opening a shared
file never runs code: the add-on only stores and forwards JSON, and executing python nodes remains
behind the app's opt-in permission (default denied). No auto-run path was added.

## Tests

* `tests/test_embed.py` (no bpy): canonical text, hashing, migration, path mapping, names, client
  capability fallback.
* `tests/test_blender_embed.py` (real Blender; `BLENDER_EXE` launches a headless process, skipped
  otherwise): save + reopen in a fresh process, purge, duplicates, extract/embed byte round trip,
  save_pre, debounce, embedded-vs-file conflict and resolution, mode switching, relative paths and
  moved projects, schema migration and newer-schema refusal, deletion cleanup, rebuild, user counts,
  legacy linked modifiers, panel drawing, freeze; plus an end-to-end test against the Rust
  `link_selftest` (push, app edit written back to the text, both conflict answers).
* Rust: `graph_embed` unit tests (sha256 vectors, push validation and limits, migration, decisions,
  state payload) and `link_instances` test (ack, hash rejection, open-document conflict, pull).

## Follow-ups

* **Pack referenced files**: not implemented. Packing images/USD into the `.blend` and rewriting the
  graph to unpack on load needs a temp-file strategy for the Rust engine; documented as follow-up.
* GN placements embed and sync back, but *Open in MiniHou* for a placement does not negotiate
  conflicts yet (it uses the existing live edit), and Freeze result is stack-modifier only.
* The app keeps one document per window; "Open embedded as new document" is implemented as
  save-mine-then-adopt rather than a second tab.
