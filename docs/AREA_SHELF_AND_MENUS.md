## Purpose
Node creation catalogues, shelf toolbar categories, Tab search menu, and contextual pie menus.

## Key files
- `rust/crates/minihou-app/src/ui/shelf_data.rs`: Static tool definitions, category tabs, and node icons.
- `rust/crates/minihou-app/src/ui/network_menu_data.rs`: Category hierarchies and node metadata for Tab search.
- `rust/crates/minihou-app/src/ui/network_menu.rs`: Tab menu search filtering, pie menu, and node spawning.
- `rust/crates/minihou-app/src/ui/shelf.rs`: Shelf bar rendering and drag-and-drop tool spawning.

## Key functions or types
`Tool`, `Tab`, `Category`, `MenuState`, `Item`, `Choice`, `show`, `dropped_tool`, `footer_text`

## Conventions to follow
- Keep node types and human-readable labels synchronized between shelf data and network menu data.
- Search fuzzy matching checks tool labels, identifiers, keywords, categories, and docstrings.
- Spawning a node through menus emits `UiAction` to place and connect the new node card.

## Tests to run for changes here
- `cargo test -p minihou-app -- shelf`
- `python -m unittest tests.test_shelf`

## Known gotchas and open TODOs
- Clear non-node tool markers to avoid showing "no Rust version" red dots ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L133)).
- Stage 1 self-registering node architecture to eliminate static catalogue edits ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L168)).
- Houdini shelf gap audit for Model, Polygon, Deform, and Texture shelves ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L191)).
