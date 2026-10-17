# Rust value linking UI — morning report

## Built

The Parameters panel now has a right-click menu on parameter labels and fields: Copy Parameter, Paste Relative Reference, Paste Value, Paste Expression, Link to Parameter, Delete Channel, Revert to Default, Lock/Unlock Parameter, Edit Expression, Go to Source, and Go to Target. The picker searches nodes and parameters and disables incompatible types. Floating sources linked to integer targets use `round(ch(...))`; vector references create three component expressions in one undoable `ChannelGroup` edit, retaining unrelated cook cache entries. Delete Channel stores the evaluated result as the literal fallback. Linked fields show the evaluated value with a green tint and a reference mark, block direct editing, and show their expression and value on hover. Locks show a lock icon. The expression dialog shows current diagnostics; broken references appear in the error list and can be retargeted with the picker.

The Network editor draws dashed cyan dependency curves behind node cards and red curves for missing sources. Selection and hovering highlight curves and reveal parameter names. Cards show reference counts or a warning badge. The Links header control and Interface preference show or hide the curves. Node and parameter menus list source and target navigation. Navigation selects and centres the destination card and flashes its parameter row.

Node context menus now offer Rename. A rename rewrites channel paths in the same graph transaction. Copy/paste remaps channel paths between copied nodes and retains external targets. Deletion leaves expressions intact so broken references remain visible and can recover if the source reappears.

## Gestures and keys

- Right-click a parameter label or field for value linking actions. Copy a parameter, select another node, then choose Paste Relative Reference on a compatible row. Link to Parameter is a searchable alternative.
- Right-click a node card for Rename, Go to Source, and Go to Target entries.
- `Shift+J` goes to the first source of the selected node; `Shift+K` goes to its first target. `Shift+D` toggles dependency curves. The Links control in the network header does the same.
- Ordinary node copy/paste remains `Ctrl+C` and `Ctrl+V` while the network has focus. Parameter copying uses the parameter context menu, so the two clipboards do not conflict.

## Validation

Focused tests cover relative paths and type compatibility, vector and scalar copy/paste, integer rounding, channel baking with undo/redo, missing-reference diagnostics, rename and paste rewriting, Go to source/target resolution, link curve geometry/hit testing, egui parameter context menu and green field tint, and keymap table synchronisation. The final workspace suite is recorded in the handoff response.

## Limits and open questions

- The selected parameter flash and card centring work in the current network level. Nested subnet navigation awaits subnet support in the Rust graph editor.
- The expression dialog edits the whole parameter channel key. Component expressions remain visible in the tooltip and can be replaced using a component-aware reference paste; a component-specific expression editor is a follow-up.
- A source/target shortcut chooses the first matching link when several exist; the menus list each destination.
- Red curves end near the affected node when the source card is missing. Invalid expressions without a readable `ch()` path are listed in the panel but have no source curve.
- Ctrl+G grouping is pending the inline subnet cooking, port extraction, and nested editing described in `DESIGN_VALUE_LINKING.md`.
- Structural node edits (rename, paste, delete) still use graph replacement and clear the cook cache; parameter menu edits use incremental session setters.
- The editor behavior has egui harness coverage but has not had a manual desktop smoke test in this worktree.

Shared files changed in small scopes: `main.rs` dispatches link navigation, rename, visibility persistence, and atomic snapshot edits; `keymap.rs` adds three network commands; `prefs.rs` persists one visibility boolean. The Blender add-on, viewport renderer, and `link_*.rs` were not changed.
