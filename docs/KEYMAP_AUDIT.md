# Pre-Rust editor keymap and gesture audit

Source: `minihou/web/static/` at this worktree revision. “Done” means the native action exists, while “added now” identifies this change. A missing row names a native gap. Tool-specific shortcuts are not applicable where the corresponding web tool or pane does not exist in the native editor. The old network shake samples 18 px horizontal chunks and needs three reversals within 650 ms (`editor.js:2563-2567`).

| Old key / gesture | Context | Behaviour | Where in old code | Native status |
| --- | --- | --- | --- | --- |
| Ctrl+Z | Global | Undo graph edit | `editor.js:2740` | done |
| Ctrl+Shift+Z / Ctrl+Y | Global | Redo graph edit | `editor.js:2740-2741` | done |
| Ctrl+S | Global | Save graph | `editor.js` file actions | done |
| Ctrl+Shift+S | Global | Save graph as | `editor.js` file actions | done |
| Ctrl+O | Global | Open graph | `editor.js` file actions | done |
| F10 | App menu | Open/close top menu | `editor.js:3310` | missing: native top bar has no keyboard menu focus |
| Arrow keys / Home / End / Escape / Tab | App menu | Navigate menus, choose edge, close | `editor.js:3312-3321` | not applicable: native egui menu focus differs |
| ? | Global | Toggle help | `editor.js:2750` | missing: no native help overlay |
| Ctrl+Space | Any pane | Maximize/restore pane | `editor.js:3121-3122` | missing: native pane maximization absent |
| Arrow keys / Home / End | Preferences tabs | Switch tabs | `editor.js:1993` | not applicable: native tab widgets provide their own focus handling |
| Escape | Popup/dialog | Close active popup | `editor.js:70,1406,2936` | done |
| Arrow Up/Down / Enter / Escape | File browser | Navigate, open, close | `editor.js:2942-2945` | not applicable: native uses OS file picker |
| Enter / Tab / Escape | Parameter expression field | Apply, indent, revert | `editor.js:1009-1012` | not applicable: native parameter widgets do not expose web expression editor |
| Enter / Escape | Node rename field | Accept/cancel rename | `editor.js:804` | missing: native node rename absent |
| Shift-drag / Ctrl-drag parameter label | Parameter | Scrub finely / quickly | `editor.js:1055-1063` | missing: native parameter label scrub absent |
| Tab | Network | Open add-node menu | `editor.js:2736` | done |
| Double-click empty canvas | Network | Open add-node menu | `editor.js:2697` | added now |
| Right-click canvas/node | Network | Open context menu | `editor.js:2698` | done |
| Arrow Up/Down / Left/Right / Enter / Escape | Add-node menu | Navigate, fold category, pick, close | `editor.js:2722-2728` | done |
| C | Network | Modeling pie menu | `pie.js:239-255` | done |
| C held, flick and release | Network pie | Pick radial category/tool | `pie.js:239-255` | missing: native pie supports click and number but no flick release |
| 1–8 / Escape | Network pie | Pick slot / close | `pie.js:242-244` | done |
| F | Network | Frame selection or graph | `editor.js:2747` | done |
| L | Network | Tidy layout | `editor.js:2748` | done |
| O | Network | Add network box around selection | `editor.js:2749` | missing: native network boxes absent |
| Ctrl+F | Network | Find node | `editor.js:3228` | missing: native node-search dialog absent |
| Ctrl+A | Network | Select all nodes | `editor.js:2745` | added now |
| Ctrl+C | Network | Copy selected nodes | `editor.js` clipboard | added now |
| Ctrl+V | Network | Paste nodes and select copies | `editor.js` clipboard | added now |
| Ctrl+D | Network | Duplicate selected nodes | `editor.js:2743` | added now |
| Ctrl+X | Network | Dissolve selected nodes; reconnect consumers to first input | `editor.js:2742` | added now |
| X | Network | Delete selected nodes | `editor.js:2746` | added now |
| Delete / Backspace | Network | Delete selected nodes or selected wire | `editor.js:2746`, help table | added now: nodes; missing: wire selection/delete absent |
| Ctrl+G | Network | Collapse selection to reusable library node | `editor.js:2744` | not applicable: native library-node authoring absent |
| M | Network | Mute selected nodes | `editor.js:2733` | added now |
| E | Network | Template selected nodes | `editor.js:2733` | added now |
| W | Network | Selectable-template flag | `editor.js:2733` | missing: native flag model has template only |
| R / Shift+R | Network | Set display/render flag | `editor.js:2733` | done: display; missing: separate render flag absent |
| Click node | Network | Select one node | `editor.js:2514-2523` | done |
| Shift/Ctrl+click node | Network | Toggle node in selection | `editor.js:2516-2520` | added now |
| LMB drag empty canvas | Network | Box select nodes | `editor.js:2531` | added now |
| Shift+LMB drag empty canvas | Network | Add box contents to selection | `editor.js:2531` | added now |
| Drag selected card | Network | Move selected cards | `editor.js:2524,2560-2562` | added now |
| Shake dragged card | Network | Detach all wires | `editor.js:2563-2567` | added now: disconnects all wires immediately and shows card toast |
| Alt+drag card | Network | Detach node and reconnect stream on release | `editor.js:2524-2525,2641` | missing: native Alt-drag detach absent |
| Escape during drag | Network | Cancel wire/card/box gesture | `editor.js:2712-2713` | missing: native card drag cancel absent |
| Shift+right-drag across wires | Network | Cut crossed wires | `editor.js:2492,2632` | missing: native wire cutting absent |
| MMB drag / Alt+LMB background drag | Network | Pan graph | `editor.js:2494` | added now: Alt+LMB; done: MMB |
| Wheel / Ctrl+wheel | Network | Zoom around pointer / faster zoom | `editor.js:2699-2704` | added now |
| Trackpad two-axis scroll | Network | Pan graph | `editor.js:2701` | done |
| Drag output to input | Network | Connect nodes | `editor.js:2498-2507` | missing: native port drag absent; context menu connect exists |
| Drag connected input / drop empty | Network | Re-route or disconnect input | `editor.js:2503-2507` | missing: native port drag absent; context menu disconnect exists |
| Drag output to empty canvas | Network | Add a connected node | `index.html:311-312` | missing: native wire-drop menu absent |
| Click wire, Delete | Network | Disconnect selected wire | `index.html:314` | missing: native wire hit selection absent |
| Click flag / Shift-click display / Ctrl-click template | Network | Set, add display, or selectable template | `editor.js:2510-2512` | done: display and Shift display; missing: selectable template |
| Double-click card | Network | Display node | `editor.js:2687` | done |
| Double-click system node / breadcrumb / Alt+Up | Network group | Enter group, navigate back/up | `editor.js:2691,2719` | not applicable: native group nesting absent |
| Drag network-box title/corner; double-click title | Network box | Move, resize, rename box | `index.html:335` | not applicable: native network boxes absent |
| Ctrl+F / Enter / Arrow Up/Down / Escape | Node search | Open, pick result, navigate, close | `editor.js:3215-3228` | missing: native node-search dialog absent |
| 1 / 2 / 3 / 4 | Viewport | Point / edge / face / object selection | `editor.js:1898-1913`, `index.html:317` | done |
| Escape | Viewport | Clear selection / leave active mode | `index.html:317` | done |
| A | Viewport | Select all components/objects | `editor.js:1898-1913` | done |
| B / C | Viewport | Box / circle selection | `selection-tools.js:38-56` | done: box; missing: circle gesture UI |
| Shift-click / Shift-box | Viewport | Add selection | `index.html:317`, `selection-tools.js:48` | done |
| Ctrl-click / Ctrl+Shift-click | Viewport | Select path / subtract | `index.html:317`, `selection-tools.js:48` | missing: path selection and Ctrl+Shift subtract |
| Click object | Viewport object mode | Select displayed object's node | `index.html:317` | added now |
| X / Delete | Viewport object mode | Delete selected object's node(s) | `editor.js:2746`, `index.html:338` | added now |
| X | Viewport point/edge/face mode | Insert a blast node for selected components | `editor.js:1497-1518,1913` | missing: Rust graph delete node cannot address component IDs |
| G / R / S | Viewport | Modal move / rotate / scale | `editor.js`, `index.html:333` | missing: concurrent modal-transform work owns these handlers; current branch selects tools |
| E / I / Ctrl+B / Alt+E | Viewport | Extrude / inset / bevel / push-pull | `editor.js:1885-1886` | done: commands; missing: web-style modal drag interaction |
| Ctrl+R | Viewport | Loop cut | `editor.js:1888`, `loop-tool.js:13-20` | missing: native modal loop-cut UI absent |
| Alt+N | Viewport | Flip selected faces | `editor.js:1889` | missing: native command has no handler |
| K / Shift+K | Viewport | Start knife / knife from selection | `editor.js:1268` | not applicable: native knife tool absent |
| 5 | Viewport | Paint mode | `editor.js:1898-1913` | missing: native paint command has no handler |
| F / Home / Numpad . | Viewport | Frame selection | `editor.js:1893-1894`, `index.html:321` | done |
| Numpad 1 / Ctrl+1 | Viewport | Front / back view | `editor.js:1891-1892` | done: Alt+1 fallback in egui |
| Numpad 3 / Ctrl+3 | Viewport | Right / left view | `editor.js:1891-1892` | done: Alt+3 fallback in egui |
| Numpad 7 / Ctrl+7 | Viewport | Top / bottom view | `editor.js:1891-1892` | done: Alt+7 fallback in egui |
| Numpad 5 / 9 | Viewport | Toggle projection / opposite view | `editor.js:1890,1893-1894` | done: Alt+5/9 fallback in egui |
| Space / Left / Right / Shift+Left / Shift+Right | Viewport timeline | Play/pause, step, jump to ends | `editor.js:1903`, `index.html:318` | done |
| MMB drag / Shift+MMB / Ctrl+MMB | Viewport | Orbit / pan / dolly | `index.html:321` | done |
| Wheel | Viewport | Zoom | `index.html:321` | done |
| RMB hold + WASD, Q/E, Shift/Ctrl, wheel | Viewport fly | Look and fly; vertical move, speed modifiers | `index.html:319` | not applicable: native fly navigation absent |
| Shift+backtick / Enter / Escape | Viewport fly | Toggle / accept / restore fly view | `index.html:319` | not applicable: native fly navigation absent |
| Drag transform gizmo; Shift/Ctrl | Viewport | Move/rotate/scale; fine/snap modifiers | `index.html:316,333`, `curve-tool.js:43-44` | done: gizmo; missing: all web modifier variants |
| LMB drag / MMB or Ctrl drag / wheel | Box/circle selection tool | Add/replace / subtract / adjust radius | `selection-tools.js:38-56` | done: box; not applicable: circle tool absent |
| Escape / Enter | Box/circle selection tool | Cancel / finish | `selection-tools.js:53` | done: box; not applicable: circle tool absent |
| [ / ] | Paint tool | Decrease/increase brush radius | `paint-tool.js:545-548` | not applicable: native paint tool absent |
| Shift / Ctrl while painting | Paint tool | Erase / smooth | `paint-tool.js:275` | not applicable: native paint tool absent |
| LMB drag / Escape | Paint tool | Stroke / cancel stroke or leave paint mode | `paint-tool.js:538-548` | not applicable: native paint tool absent |
| LMB clicks / RMB / Escape | Sketch tool | Place point / commit chain / cancel | `sketch-tool.js:69-73`, `index.html:329` | done: basic sketch; missing: right-click chain flow |
| Enter / Backspace / X/Y/Z / digits / . | Sketch tool | Finish, undo point, constrain axis, enter length | `sketch-tool.js:73` | missing: native sketch numeric/axis controls absent |
| Click measurement label / Enter | Sketch measurement | Edit saved measurement | `sketch-tool.js:60` | missing: native measurement editing absent |
| Drag curve anchor/tangent; Shift/Ctrl | Curve tool | Adjust control, precision/snap | `curve-tool.js:29-49` | not applicable: native Bezier control editor absent |
| X/Y/Z / Shift+X/Y/Z / Enter / Escape | Curve modal drag | Axis/plane constraint, accept/cancel | `curve-tool.js:48` | not applicable: native Bezier control editor absent |
| Shift+double-click | Curve tool | Add control point | `curve-tool.js:52` | not applicable: native Bezier control editor absent |
| LMB / MMB / RMB / double-click | Knife tool | Add/drag cut, new path, close path | `knife-tool.js:30-33` | not applicable: native knife tool absent |
| Enter / Space / Escape / Ctrl+Z / Backspace | Knife tool | Commit, cancel, undo last segment | `knife-tool.js:34-35` | not applicable: native knife tool absent |
| C / V / S / A / R / E / X/Y/Z / digits | Knife tool | Through, x-ray, measure, angle, reference, new path, axis, angle increment | `knife-tool.js:34-35` | not applicable: native knife tool absent |
| LMB / RMB / wheel / Ctrl drag | Loop cut tool | Start/confirm, center, count, snap slide | `loop-tool.js:13-20` | not applicable: native modal loop-cut tool absent |
| Enter / Escape | Loop cut tool | Accept / cancel | `loop-tool.js:19` | not applicable: native modal loop-cut tool absent |
| Drag / Shift / Ctrl | Modal modeling tool | Adjust amount finely / snap | `modal-model-tool.js:28` | not applicable: native web modal modeling tool absent |
| Digits / . / - / Backspace / Enter / Escape | Modal modeling tool | Type amount, edit, accept/cancel | `modal-model-tool.js:33-35` | not applicable: native web modal modeling tool absent |
| Wheel | Modal bevel | Adjust divisions | `modal-model-tool.js:37` | not applicable: native web modal modeling tool absent |
| Arrow Down/Up / Escape | Viewport popover | Open/move focus/close | `viewport-ui.js:37-56` | done: egui popovers handle focus and close |
| Enter / Escape | Sheet filter | Apply / clear filter | `sheet.js:32` | not applicable: native sheet pane absent |
| Alt+click column / Shift+click cell | Sheet | Alternate sort / extend cell selection | `sheet.js:106,192` | not applicable: native sheet pane absent |
| Shift/Ctrl+click row / double-click row | Sheet | Add row to selection / frame element | `sheet.js:269-271` | not applicable: native sheet pane absent |
| Enter / Escape | Sheet cell edit | Commit / cancel | `sheet.js:287` | not applicable: native sheet pane absent |

The native keymap table is in `rust/crates/minihou-app/docs/NATIVE_EDITOR.md`. Pointer gestures live in panel handlers because egui does not route them through `Keymap`.
