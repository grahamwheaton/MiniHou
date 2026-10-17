# Blender Object node (add-on 0.5.0)

The `blender_object` node reads a Blender object's evaluated mesh into a MiniHou graph. Since add-on 0.5.0 it
works three ways and never shows a red "failed" node just because nothing has been chosen yet.

## 1. Object picker on the MiniHou group

When the graph is bound to Blender (a MiniHou modifier, a MiniHou Geometry Nodes placement, embedded or
linked-file network), every `blender_object` node is an **implicit object input**: the Blender group gets a
native Object socket (ID field with eyedropper) named after the node, e.g. `blender_object1`, on the MiniHou
Plumbing group node, the modifier panel and each placement's "MiniHou N" section.

* It reuses the dynamic-input machinery (`modifier_inputs::interface_entries`, add-on
  `input_define.interface_entries`): the interface entry is `{"node":"@picker","id":<stable id>,"kind":"object",
  "implicit":true}` and the pick travels as the `input:@picker:<id>` source with name + world matrix, exactly
  like an object Add Input. Picker entries are not parameter-link targets and are not listed with Rename/Remove.
* Stable id: the node id, pinned in the node's `picker_id` field on its first rename (`graph_edit::rename`), so
  the Blender socket identifier, its value, links and drivers survive renames. A pasted copy with a duplicate
  `picker_id` falls back to its own node id. Removing the node removes the socket.
* In a bound cook the node reads only its picker (`picker:<node>` in the session's external store); an empty
  picker means *waiting*. A pick (or clear) also writes the object's name to the node's `object` param through
  the normal parameter path, so the graph still works by name standalone or in the linked file. The add-on's
  source validation refuses picking the modifier's own object or a MiniHou output (no feedback loops); modifier
  graphs may therefore contain `blender_object` nodes now.
* Per placement: each placement's socket value is its own feed. Old files need no migration: the sockets
  appear on the next cook with the app.

Not bound (the app's own document with no Blender modifier live): the node reads the object named in `object`
through `watch_objects`, as before.

## 2. Waiting, not failed

`Error::waiting(msg)` marks "waiting for external data". The cook gives the node empty geometry, downstream nodes
cook normally, and `CookStats::waiting` lists `(node, message)` for every waiting external node (also when served
from the cache). The editor draws such nodes with an amber outline, status `waiting` and a quiet note
("no object picked: pick an object in Blender"); the Parameters panel shows a neutral note; the status line says
`Waiting: blender_object1 (...)`; nothing is reported as a cook error. Instance results carry `cook.waiting`.

Converted conditions: `blender_object` (no object set or picked, Blender not connected, "no object named ..."),
`blender_image` (no image set, not connected, "no image named ..."), `object_input` (empty picker). Real errors
stay red: a picked object that cannot be read, a MiniHou output named as the object, invalid pixel data.

## 3. Fuzzy object-name search

The `object` field in the Parameters panel searches Blender object names while it is being edited
(`object_names_v1`, advertised in the add-on's hello):

* App -> add-on `object_names_query {request_id, query, limit<=50}`: sent on focus and 120 ms after the last
  keystroke, never otherwise. The UI keeps showing the last results (re-ranked locally at once) with
  "searching...", never waits, and discards responses whose request id is not the newest. Answers are cached by
  (scene hash, query).
* Add-on -> app `object_names {request_id, query, hash, count, count_exact, partial, scanned, ms, matches:
  [{name, type, mesh}]}`: at most 50 ranked matches; the full list never crosses the link.
* The add-on scans on its timer: a held `bpy.data.objects` iterator (revalidated each tick: undo/load
  generation, and the last object read must still be alive), about 4 ms per tick, names cached per scene
  signature (depsgraph object updates and undo/load invalidate it). A newer query replaces the running scan; a
  partial answer is sent after 150 ms; the scan gives up with `partial: true` after 3 s. An empty query reads
  only the first 50 names (ID lists are sorted).
* Scoring (`object_names::score`, mirrored in Python): exact > prefix > substring (word-start bonus) >
  case-insensitive subsequence with contiguity and word-start bonuses; shorter names win ties.
* Older add-ons (no capability) leave the field a plain text box.

Measured in headless Blender 5.2 (empty objects; `tests/test_blender_object_picker.py`): a cold search over
100k objects takes ~150 ms of main-thread time spread over ~36 ticks (largest tick ~4.6 ms); repeat queries use
the name cache (~120 ms over ~28 ticks); an empty query takes ~0.5 ms. 10k: ~18 ms over 5 ticks; 1k: ~3 ms.
