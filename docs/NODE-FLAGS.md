# Geometry node flags

MiniHou uses Houdini-style geometry (SOP) flags. These are separate from Blender object visibility.

| Flag | Control | Behavior |
| --- | --- | --- |
| Mute | Yellow M button; M | Passes the first declared input through without running the operation or cooking other inputs. A generator produces empty geometry. |
| Lock / Freeze | Red L button; context menu | Saves the current full-resolution geometry in the graph. Parameters and upstream changes have no effect until unlocked. |
| Template | Maroon T button; E | Reference wireframe and snapping, excluded from output and component selection. Normally exclusive; Shift-click adds references. |
| Selectable Template | Ctrl-click Template; W | Adds shaded, selectable reference geometry. Applying a selection tool or gizmo inserts a Merge node before the operation and clears the consumed template flags. |
| Display | Blue D button; R | Chooses the viewport/live-link output. Clicking it sets the preview source to Display flag. The preview source dropdown can inspect a selected node independently. |
| Unload | Context menu; U badge | Does not retain intermediate geometry in memory or write it to disk cache. Directly displayed/cooked targets remain cached. |

Shortcuts apply in the network, not while typing or using viewport transform tools.
Mute, Lock and Unload act on the selected nodes when clicked on a selected node.
Flags survive save/reload, duplication, clipboard operations and undo/redo.
Try `examples/node_flags.json`.

## Network interactions

The viewport defaults to **Display flag**. Selecting or box-selecting nodes does not change it.
Use **Inspect selected node** only when you explicitly want the viewport to follow selection.
The Render button, shortcut and menu entry have been removed. Older graphs retain their
render metadata for compatibility with existing scripts; new graphs use Display.

- Left-drag empty space: box select; Shift adds to the selection.
- Middle-drag or Alt-left-drag: pan.
- X, Delete or Backspace: delete selected nodes or wire; Ctrl-Z undoes.
- Shake dragged nodes horizontally (three quick direction reversals): disconnect external
  wires on release, preserving wires between selected nodes. Ordinary movement does not disconnect.
- Shift-right-drag across wires: cut crossed wires on release. Crossed wires turn red.
- Escape cancels a pending move, shake, box selection or cut. Undo restores a completed gesture.

## Limits

Locks are compressed, non-pickle snapshots stored in graph JSON (64 MiB uncompressed
per node); unsupported non-JSON detail attributes are rejected. This can increase graph
file size. Unlock to resume live Blender inputs or simulation. Mute takes precedence
over Lock. Bypass a Loop End to skip its whole repeat block; a Loop Begin alone cannot
be bypassed. Templates use the viewport geometry budget; their display style is currently
wireframe, with shaded geometry for selectable templates. No object-level X-ray or
display-origin flags, Houdini node ring, or hold-key-and-click flag gestures are included.

Behavior reference: https://www.sidefx.com/docs/houdini/network/flags.html


## Modelling and layout update

Ctrl-X dissolves selected nodes and reconnects consumers through their first input.
Multi-input consumers keep all resolved sources; single-input consumers use the first.
Plain X deletes without reconnecting. Old graphs retain the internal `bypass` key for Mute.

Blast invoked from the shelf or viewport fills Group and Domain from the current selection.
Edge Blast removes incident faces (it is not an edge dissolve). Primitive drops into the 3D
view place their centre on the XY construction plane and flag the new result for display.

Box supports optional bounding geometry, quadrilateral/triangle connectivity, surface points,
corner consolidation, per-axis cell divisions, rotation, uniform scale and vertex normals.
Zero axis divisions preserve the old uniform Divisions value. Sphere, Tube, Torus and Grid
also expose rotation, vector scale and uniform scale around Center. This is not full Houdini
primitive parity: no NURBS/Bezier surfaces, solid primitives, interior lattice/enforcement bars,
PCA oriented bounds, object parenting/pivot hierarchy, or separate object Render/Misc tabs.

Move gizmos provide XY/XZ/YZ plane squares alongside axis arrows, with existing precision,
snapping, orientation and Escape cancellation. This does not claim full Blender gizmo parity.
WASD fly navigation was not found in the available pushed branches; it is not in this build.
