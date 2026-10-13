# Geometry node flags

MiniHou uses Houdini-style geometry (SOP) flags. These are separate from Blender object visibility.

| Flag | Control | Behavior |
| --- | --- | --- |
| Bypass / Ignore | Yellow left strip; Q or B | Passes the first declared input through without running the operation or cooking other inputs. A generator produces empty geometry. |
| Lock / Freeze | Red L strip; context menu | Saves the current full-resolution geometry in the graph. Parameters and upstream changes have no effect until unlocked. |
| Template | Maroon strip; E | Reference wireframe and snapping, excluded from output and component selection. Normally exclusive; Shift-click adds references. |
| Selectable Template | Ctrl-click Template; W | Adds shaded, selectable reference geometry. Applying a selection tool or gizmo inserts a Merge node before the operation and clears the consumed template flags. |
| Display | Blue strip; R | Chooses the viewport/live-link output. Clicking it sets the preview source to Display flag. The preview source dropdown can inspect a selected node independently. |
| Render | Purple strip; T or Ctrl-click Display | Separate output for USD export, CLI `cook --render`, and explicit Blender render/bake preparation. Toggle off to follow Display again. |
| Unload | Context menu; U badge | Does not retain intermediate geometry in memory or write it to disk cache. Directly displayed/cooked targets remain cached. |

Shortcuts apply in the network, not while typing or using viewport transform tools.
Bypass, Lock and Unload act on the selected nodes when clicked on a selected node.
Flags survive save/reload, duplication, clipboard operations and undo/redo.
Try `examples/node_flags.json`.

## Render integration and limits

The Blender instance protocol accepts `render=True`. The add-on's existing synchronous
`modifiers.cook_frame(scene, frame)` and RBD bake preparation use this output;
ordinary viewport polling uses Display. For scripted still renders, call the barrier
immediately before the render operator as in the existing RBD workflow.
Automatic switching for Blender's ordinary F12/animation render and native nested
Geometry Nodes wrappers is not implemented. MiniHou must not change Blender datablocks
inside active rendering. Do not rely on the purple flag alone to change an F12 render.

Locks are compressed, non-pickle snapshots stored in graph JSON (64 MiB uncompressed
per node); unsupported non-JSON detail attributes are rejected. This can increase graph
file size. Unlock to resume live Blender inputs or simulation. Bypass takes precedence
over Lock. Bypass a Loop End to skip its whole repeat block; a Loop Begin alone cannot
be bypassed. Templates use the viewport geometry budget; their display style is currently
wireframe, with shaded geometry for selectable templates. No object-level X-ray or
display-origin flags, Houdini node ring, or hold-key-and-click flag gestures are included.

Behavior reference: https://www.sidefx.com/docs/houdini/network/flags.html
