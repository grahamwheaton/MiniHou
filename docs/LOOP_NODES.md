# For Each zones

Add For Each creates a `for_each_start`, a `for_each_end`, and one `for_each`
zone record in the current graph scope. The end's Generated input is wired to
the start's Element output, and its Begin input identifies the matching start.
The zone owns its member IDs; nodes remain in the scope's `nodes` map. The
legacy single-node `for_each` format still loads.

The start's Method is Connected, Attribute, or Domain. Domain offers Points
(the default), Edges, Faces, and Instances. Attribute partitioning accepts
point and instance attributes as well as edge and face attributes. The Index
socket carries an integer source index, and the body is cooked once per
selected element. Repeat feedback zones remain separate.

## Follow-up

- TODO: Add Blender-style full main-geometry outputs around the body. This
  needs a separate output contract and merge policy for untouched elements and
  instances across all four domains.
- TODO: Add body inspection that selects an iteration and displays its input,
  generated output, and source index in the live editor. This needs editor
  state and cache navigation beyond the frame and sockets implemented here.

The framed editor and menu action have automated coverage; interaction,
layout, and visual behavior still need a live app check.
