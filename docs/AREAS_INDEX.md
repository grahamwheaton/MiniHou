# MiniHou Codebase Areas Index

## Owner Rule
**One agent or worktree edits an area at a time.**
To prevent merge conflicts, preserve architectural boundaries, and ensure concurrent multi-agent safety, each area map defines an exclusive boundary of responsibility. Before modifying files in an area, claim ownership and verify no other worktree or agent is actively editing it.

## Areas

| Area | Purpose | Area Map |
|---|---|---|
| `NODE_EDITOR` | Node graph canvas rendering, node cards, wire routing, spring physics, and layout | [docs/AREA_NODE_EDITOR.md](AREA_NODE_EDITOR.md) |
| `PARAMETERS` | Parameter inspector widgets, numeric scrubbing, unit conversion, and param resolution | [docs/AREA_PARAMETERS.md](AREA_PARAMETERS.md) |
| `VIEWPORT` | GPU 3D viewport rendering, camera navigation, display modes, and WGSL shaders | [docs/AREA_VIEWPORT.md](AREA_VIEWPORT.md) |
| `MODAL_TOOLS` | Interactive viewport modeling (extrude, inset, bevel), cut/slice tools, and transform modals | [docs/AREA_MODAL_TOOLS.md](AREA_MODAL_TOOLS.md) |
| `SKETCH_INFERENCE` | 2D/3D sketching interaction, constraint solving, guide lines, and smart snapping | [docs/AREA_SKETCH_INFERENCE.md](AREA_SKETCH_INFERENCE.md) |
| `SHELF_AND_MENUS` | Node creation catalogues, shelf toolbar categories, Tab search menu, and pie menus | [docs/AREA_SHELF_AND_MENUS.md](AREA_SHELF_AND_MENUS.md) |
| `MESH_NODES` | Core procedural mesh generation, polygon modeling nodes, and boolean CSG operations | [docs/AREA_MESH_NODES.md](AREA_MESH_NODES.md) |
| `DEFORMERS` | Geometric deformers, axial bends, twists, tapers, point relax/jitter, and lattices | [docs/AREA_DEFORMERS.md](AREA_DEFORMERS.md) |
| `TEXTURES_MATERIALS` | Texture asset registry, material descriptors, image transfer service, and GPU textures | [docs/AREA_TEXTURES_MATERIALS.md](AREA_TEXTURES_MATERIALS.md) |
| `BLENDER_ADDON` | Blender integration add-on, modifier bridges, geometry streaming, and viewport UI | [docs/AREA_BLENDER_ADDON.md](AREA_BLENDER_ADDON.md) |
| `LINK_AND_MCP` | Inter-process link server, binary streaming protocol, HTTP REST API, and MCP server | [docs/AREA_LINK_AND_MCP.md](AREA_LINK_AND_MCP.md) |
| `BUILD_AND_TESTS` | Compilation recipes, test commands, concurrency tuning, and environment variables | [docs/AREA_BUILD_AND_TESTS.md](AREA_BUILD_AND_TESTS.md) |
