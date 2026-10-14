# Modal modeling gestures

The editor supplies node creation, scalar parameter updates, graph evaluation, and undo/cancel hooks to `MiniModalModelTool`. Bevel, Inset, Extrude, and Push/Pull share the pointer lifecycle. Offset is accumulated from horizontal minus vertical pointer motion; a returned context may specify `scale` (world units per CSS pixel), with 0.01 as the default. Shift reduces subsequent movement to one tenth for continuous precision. Ctrl rounds the displayed/applied offset to 0.1 units. Bevel uses the wheel to change divisions between 1 and 64.

Click or Enter confirms. Escape or right click cancels. Startup and updates may be asynchronous: an early release does not confirm, cancellation waits for the context and any in-flight update, and confirmation drains the latest offset before finalizing. Text fields retain keyboard input. These gestures supply a scalar offset interface; they do not reproduce every Blender modal operation option.
