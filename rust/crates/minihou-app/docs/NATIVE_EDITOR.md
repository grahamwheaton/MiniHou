# MiniHou native editor

Open a graph JSON file by passing its path to `minihou-app.exe`, or start the app without arguments for the demo graph. The top bar opens and saves files, changes shading, and opens Preferences. The shelf adds tools to the selected node; drag a tool into the network or viewport to add it there. The network editor selects, connects, duplicates, deletes, copies, and pastes nodes. The right panel edits parameters and shows geometry information. The centre viewport supports orbit, pan, zoom, mesh picking, view presets, and display proxy levels. The bottom strip shows cook state, file messages, and the Blender triangle cap when set.

Click a shelf tool to add it after the selected node. In the network editor, `Ctrl+C` copies the selected node and `Ctrl+V` pastes it with a fresh ID. The internal wire mapping is preserved when a copied set contains more than one node. Drag from a node output to an input to connect it; use a node menu to disconnect an input.

Preferences are available from the top bar or `Ctrl+,`. The Viewport tab contains the Auto proxy triangle budget. The viewport proxy menu has the Blender triangle cap; zero means unlimited. Preferences are stored in `%APPDATA%\MiniHou\preferences.json` on Windows.

## Default keymap

The table below is checked against `Keymap::blender()` by a unit test. Shortcuts in the Network context work while the pointer is over the network editor; viewport shortcuts work over the viewport.

<!-- keymap:start -->
| Shortcut | Command | Context |
| --- | --- | --- |
| 1 | Points | Viewport |
| 2 | Edges | Viewport |
| 3 | Faces | Viewport |
| 4 | Object | Viewport |
| Q | Select tool | Viewport |
| W | Move tool | Viewport |
| E | Rotate tool | Viewport |
| R | Scale tool | Viewport |
| T | Transform tool | Viewport |
| Home | Frame all | Viewport |
| Alt+Period | Frame selected | Viewport |
| Alt+1 | Front | Viewport |
| Ctrl+Alt+1 | Back | Viewport |
| Alt+3 | Right | Viewport |
| Ctrl+Alt+3 | Left | Viewport |
| Alt+7 | Top | Viewport |
| Ctrl+Alt+7 | Bottom | Viewport |
| Alt+5 | Toggle perspective / ortho | Viewport |
| Alt+2 | Orbit down | Viewport |
| Alt+4 | Orbit left | Viewport |
| Alt+6 | Orbit right | Viewport |
| Alt+8 | Orbit up | Viewport |
| Ctrl+Alt+4 | Pan left | Viewport |
| Ctrl+Alt+6 | Pan right | Viewport |
| Ctrl+Alt+8 | Pan up | Viewport |
| Ctrl+Alt+2 | Pan down | Viewport |
| Alt+Plus | Zoom in | Viewport |
| Alt+Minus | Zoom out | Viewport |
| Alt+Z | X-ray | Viewport |
| Shift+Tab | Snapping | Viewport |
| O | Proportional editing | Viewport |
| Z | Next shading | Viewport |
| Shift+Z | Previous shading | Viewport |
| Ctrl+Z | Undo | Everywhere |
| Ctrl+Y | Redo | Everywhere |
| Ctrl+Shift+Z | Redo | Everywhere |
| Ctrl+S | Save | Everywhere |
| Ctrl+Shift+S | Save as | Everywhere |
| Ctrl+O | Open | Everywhere |
| Ctrl+Comma | Preferences | Everywhere |
| Ctrl+C | Copy nodes | Network |
| Ctrl+V | Paste nodes | Network |
| X | Delete node | Network |
| Delete | Delete node | Network |
| Shift+D | Duplicate node | Network |
| Ctrl+L | Tidy network | Network |
<!-- keymap:end -->

## Known limitations

Element transforms write a `group` of point numbers. Edge selection includes both endpoints; face selection maps each picked display triangle to its three point numbers. Object transforms leave `group` empty. Proportional radius, falloff, and connected mode are saved on the transform node when proportional editing is enabled.

The Rust core currently cooks only the nodes marked with a green dot on the shelf. Copy and paste currently operate on the single selected network node. egui does not distinguish numpad digits from the number row, so Alt+digit represents numpad keys; Preferences can emulate the numpad with plain digits. Lock and template flags are session state. Auto proxy reduces the displayed mesh; the Blender cap is a separate setting for the link.
