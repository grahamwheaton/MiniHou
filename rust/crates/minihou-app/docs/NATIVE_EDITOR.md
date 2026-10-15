# MiniHou native editor

Open a graph JSON file by passing its path to `minihou-app.exe`, or start the app without arguments for the demo graph. The top bar opens and saves files, changes shading, and opens Preferences. The shelf adds tools to the selected node; drag a tool into the network or viewport to add it there. The network editor selects, connects, duplicates, deletes, copies, and pastes nodes. The right panel edits parameters and shows geometry information. The centre viewport supports orbit, pan, zoom, mesh picking, view presets, and display proxy levels. The bottom strip shows cook state, file messages, and the Blender triangle cap when set.

Click a shelf tool to add it; tools with a required input connect after the selected node, while generators remain unwired and join the displayed set. Click a blue Visible flag to show one node, or Shift-click to add or remove it from the displayed set. The graph stores one displayed node as a string and several as a list. Tab over the network or viewport opens the add menu at the pointer; type to filter, use arrows and Enter to add, or Esc to close it. In the network editor, `Ctrl+C` copies the selected node and `Ctrl+V` pastes it with a fresh ID. The internal wire mapping is preserved when a copied set contains more than one node. Drag from a node output to an input to connect it; use a node menu to disconnect an input.

Preferences are available from the top bar. The Viewport tab contains the Blender and Houdini / MiniHou navigation styles and the Auto proxy triangle budget. Blender navigation (the default) uses middle mouse to orbit, Shift+middle to pan, Ctrl+middle to zoom, and the wheel to zoom. Houdini / MiniHou uses Alt+left to orbit, Alt+middle to pan, and Alt+right to zoom. The viewport proxy menu has the Blender triangle cap; zero means unlimited. Preferences are stored in `%APPDATA%\MiniHou\preferences.json` on Windows.

## Default keymap

The table below is checked against `Keymap::blender()` by a unit test. Shortcuts in the Network context work while the pointer is over the network editor; viewport shortcuts work over the viewport. Alt+1/3/5/7/9 and Alt+Period are numpad fallbacks because egui cannot distinguish numpad keys. Emulate Numpad uses plain 1/3/5/7/9 for views and Shift+1/2/3/4/5 for viewport modes. In the modeling menu, 1-8 choose a numbered category or tool.

<!-- keymap:start -->
| Shortcut | Command | Context |
| --- | --- | --- |
| 1 | Points | Viewport |
| 2 | Edges | Viewport |
| 3 | Faces | Viewport |
| 4 | Object | Viewport |
| G | Move (modal) | Viewport |
| R | Rotate (modal) | Viewport |
| S | Scale (modal) | Viewport |
| B | Box select | Viewport |
| C | Circle select | Viewport |
| A | Select all | Viewport |
| F | Frame selected | Viewport |
| O | Proportional editing | Viewport |
| 5 | Paint | Viewport |
| E | Extrude | Viewport |
| I | Inset | Viewport |
| Escape | Select none | Viewport |
| Ctrl+B | Bevel | Viewport |
| Alt+E | Push / pull | Viewport |
| Ctrl+R | Loop cut | Viewport |
| Alt+N | Flip faces | Viewport |
| Tab | Add node menu | Viewport |
| Home | Frame selected | Viewport |
| Space | Play / pause | Viewport |
| Left | Previous frame | Viewport |
| Right | Next frame | Viewport |
| Shift+Left | First frame | Viewport |
| Shift+Right | Last frame | Viewport |
| Tab | Add node menu | Network |
| Alt+1 | Front | Viewport |
| Ctrl+Alt+1 | Back | Viewport |
| Alt+3 | Right | Viewport |
| Ctrl+Alt+3 | Left | Viewport |
| Alt+7 | Top | Viewport |
| Ctrl+Alt+7 | Bottom | Viewport |
| Alt+5 | Toggle perspective / ortho | Viewport |
| Alt+9 | Opposite view | Viewport |
| Alt+Period | Frame selected | Viewport |
| F | Frame all | Network |
| L | Tidy network | Network |
| O | Network box | Network |
| C | Modeling pie menu | Network |
| R | Display flag | Network |
| M | Mute flag | Network |
| E | Template flag | Network |
| W | Selectable template flag | Network |
| Shift+R | Display flag | Network |
| Ctrl+Z | Undo | Everywhere |
| Ctrl+Y | Redo | Everywhere |
| Ctrl+Shift+Z | Redo | Everywhere |
| Ctrl+S | Save | Everywhere |
| Ctrl+Shift+S | Save as | Everywhere |
| Ctrl+O | Open | Everywhere |
| Ctrl+D | Duplicate node | Network |
| Ctrl+C | Copy nodes | Network |
| Ctrl+V | Paste nodes | Network |
| Ctrl+A | Select all | Network |
| Ctrl+F | Find node | Network |
| Delete | Delete node | Network |
| Backspace | Delete node | Network |
| X | Delete node | Network |
<!-- keymap:end -->

## Modal transforms

With the pointer in the viewport and an object or edit-mode elements selected, G, R, and S begin a modal transform. The object updates as the pointer moves. The bottom viewport strip shows values, constraint, and snap state. The coloured constraint line passes through the pivot. Confirm creates one undo step; cancel restores the previous graph. The existing toolbar gizmo handles remain available. The following table is checked against the modal key list by a unit test.

<!-- modal:start -->
| Input | During modal transform |
| --- | --- |
| G / R / S | Begin move / rotate / scale while the pointer is in the viewport |
| LMB / Enter / Space | Confirm |
| RMB / Escape | Cancel |
| X / Y / Z | Axis; again local axis; third press clears |
| Shift+X / Y / Z | Plane excluding axis |
| Middle mouse | Choose closest motion axis; again clears |
| Digits / . / - / Backspace | Enter exact distance, angle in degrees, or scale factor |
| Tab | Next move numeric field |
| Ctrl | Invert snapping |
| Shift | Precision movement |
| O | Toggle proportional editing |
| Wheel / PageUp / PageDown | Change proportional radius |
| B while moving | Cycle Increment / Vertex / Edge / Face snap target |
| Ctrl+Shift+LMB while moving | Pick snap reference point |
| R while rotating | Toggle trackball |
| B while rotating | Pick two-point pivot, reference, then angle |
| C while scaling | Toggle scale cage; click a handle, move, click to confirm |
| Alt while scaling | Individual origins is not yet supported for multiple displayed objects |
<!-- modal:end -->

## Known limitations

Element transforms write a `group` of point numbers. Edge selection includes both endpoints; face selection maps each picked display triangle to its three point numbers. Object transforms leave `group` empty. Proportional radius, falloff, and connected mode are saved on the transform node when proportional editing is enabled.

Modal rotation uses the nearest Cartesian axis to the view direction for its default axis. Face-normal sketch rotation, cursor orientation rotation, and scale individual origins across multiple displayed objects are not available yet. The scale cage creates `match_size` for one displayed object when it can represent the result with nonnegative scales; otherwise it keeps the transform node.

The Rust core currently cooks only the nodes marked with a green dot on the shelf. Copy and paste currently operate on the single selected network node. egui does not distinguish numpad digits from the number row, so Alt+digit represents numpad keys; Preferences can emulate the numpad with plain digits. Lock and template flags are session state. Auto proxy reduces the displayed mesh; the Blender cap is a separate setting for the link.
