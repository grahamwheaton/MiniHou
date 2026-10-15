# MiniHou native editor

Open a graph JSON file by passing its path to `minihou-app.exe`, or start the app without arguments for the demo graph. The top bar opens and saves files, changes shading, and opens Preferences. The shelf adds tools to the selected node; drag a tool into the network or viewport to add it there. The network editor selects, connects, duplicates, deletes, copies, and pastes nodes. The right panel edits parameters and shows geometry information. The centre viewport supports orbit, pan, zoom, mesh picking, view presets, and display proxy levels. The bottom strip shows cook state, file messages, and the Blender triangle cap when set.

Click a shelf tool to add it; tools with a required input connect after the selected node, while generators remain unwired and join the displayed set. Click a blue Visible flag to show one node, or Shift-click to add or remove it from the displayed set. The graph stores one displayed node as a string and several as a list. Tab over the network or viewport opens the add menu at the pointer; type to filter, use arrows and Enter to add, or Esc to close it. In the network editor, `Ctrl+C` copies the selected nodes and `Ctrl+V` pastes them with fresh IDs. Wires between copied nodes are preserved. The native canvas still lacks direct port dragging and wire selection.

Preferences are available from the top bar. The Viewport tab contains the Blender and Houdini / MiniHou navigation styles and the Auto proxy triangle budget. Blender navigation (the default) uses middle mouse to orbit, Shift+middle to pan, Ctrl+middle to zoom, and the wheel to zoom. Houdini / MiniHou uses Alt+left to orbit, Alt+middle to pan, and Alt+right to zoom. The viewport proxy menu has the Blender triangle cap; zero means unlimited. Preferences are stored in `%APPDATA%\MiniHou\preferences.json` on Windows.

Shift- or Ctrl-click nodes to toggle them in the network selection, or drag on empty canvas to box select (Shift adds). `X` and `Delete` remove the selected set from either the network or object-mode viewport. `Ctrl+X` dissolves selected nodes and reconnects their consumers to the first input source. Shake a dragged card left and right to disconnect its input and output wires. These graph changes are each one undo step.

## Default keymap

The table below is checked against `Keymap::blender()` by a unit test. Shortcuts in the Network context work while the pointer is over the network editor; viewport shortcuts work over the viewport. Alt+1/3/5/7/9 and Alt+Period are numpad fallbacks because egui cannot distinguish numpad keys. Emulate Numpad uses plain 1/3/5/7/9 for views and Shift+1/2/3/4/5 for viewport modes. In the modeling menu, 1-8 choose a numbered category or tool.

<!-- keymap:start -->
| Shortcut | Command | Context |
| --- | --- | --- |
| 1 | Points | Viewport |
| 2 | Edges | Viewport |
| 3 | Faces | Viewport |
| 4 | Object | Viewport |
| G | Move tool | Viewport |
| R | Rotate tool | Viewport |
| S | Scale tool | Viewport |
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
| Ctrl+D | Duplicate nodes | Network |
| Ctrl+X | Dissolve nodes | Network |
| Ctrl+C | Copy nodes | Network |
| Ctrl+V | Paste nodes | Network |
| Ctrl+A | Select all | Network |
| Ctrl+F | Find node | Network |
| Delete | Delete nodes | Network |
| Backspace | Delete nodes | Network |
| X | Delete nodes | Network |
| X | Delete nodes | Viewport |
| Delete | Delete nodes | Viewport |
<!-- keymap:end -->

## Known limitations

Element transforms write a `group` of point numbers. Edge selection includes both endpoints; face selection maps each picked display triangle to its three point numbers. Object transforms leave `group` empty. Proportional radius, falloff, and connected mode are saved on the transform node when proportional editing is enabled.

The Rust core currently cooks only the nodes marked with a green dot on the shelf. Copy and paste operate on the selected network set. egui does not distinguish numpad digits from the number row, so Alt+digit represents numpad keys; Preferences can emulate the numpad with plain digits. Lock and template flags are session state. Auto proxy reduces the displayed mesh; the Blender cap is a separate setting for the link.
