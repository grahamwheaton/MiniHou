# Editor UI

The application shell provides File/Edit/View/Help menus (F10 and arrow keys), a compact command toolbar, resizable panes with active-pane indicators, and a bottom status bar. Common text-entry and confirmation actions use in-app dialogs. Preferences has 3D viewport and Node editor tabs. Blender setup opens from the status indicator or View menu; it stays out of the workspace until requested. Commands omitted from the compact toolbar at smaller window sizes remain available in the menus.

Use **Preferences** in the top bar to configure browser-local viewport and node-editor colours, backdrop, snapping and display options. Changes apply immediately; the toolbar stays in sync. Presets include Default, Houdini-style, High contrast and a saved Custom preset. Export/import JSON to transfer preferences between browsers. Invalid imports leave current settings unchanged.

Right-click a node to rename, duplicate, colour, delete, frame the selection, or group it in a frame. Right-click a wire to focus either endpoint or disconnect it. Right-click empty canvas to add a node, frame or sticky note.

**Find** (Ctrl/Cmd+F outside text fields) searches node names, types and comments. Use arrow keys and Enter to choose a result. Finding a node inside a collapsed frame expands the frame.

**Frame** (O) encloses selected nodes. Drag the header to move contained nodes, drag the corner to resize, double-click the title to rename, and use the colour swatch to recolour. The frame context menu provides fit to contents, collapse/expand, layout lock/unlock, duplicate and delete. Deleting a frame keeps its nodes. Collapsing hides contained nodes and wires without changing the cooked graph. Layout locking prevents frame and contained-node drags; it does not prevent parameter edits or graph operations.

**Note** adds an editable sticky note. Text is committed when the field loses focus, and supports undo/redo. Notes and frames, including collapse/lock state and node colours, are stored in graph JSON. Browser preferences are separate from graph files.

The viewport display menu offers Shaded, Shaded + edges, Wireframe and Points. Lighting intensity, point size, automatic or explicit grid spacing, grid brightness and world axes are configurable in Preferences.
