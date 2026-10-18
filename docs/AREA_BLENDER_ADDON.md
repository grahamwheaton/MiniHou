## Purpose
Blender integration add-on, modifier bridges, geometry streaming, and viewport UI controls.

## Key files
- `blender_addon/minihou_link/__init__.py`: Add-on entry point, UI panels, operators, and preferences.
- `blender_addon/minihou_link/runtime.py`: Engine process lifecycle and background communication.
- `blender_addon/minihou_link/modifiers.py`: Custom modifier stack bindings and recook triggers.
- `blender_addon/minihou_link/mesh_writer.py`: Export Blender geometry and attributes into MiniHou streams.
- `blender_addon/minihou_link/mesh_reader.py`: Import cooked MiniHou meshes into Blender scene datablocks.

## Key functions or types
`MINIHOU_Preferences`, `MINIHOU_PT_main`, `MINIHOU_OT_connect`, `MINIHOU_OT_resync`, `MINIHOU_OT_add_modifier`, `register`, `unregister`, `is_modifier`, `instance_id`

## Conventions to follow
- Adhere strictly to Blender Python `bpy` API rules and clean registration lifecycles.
- Handle engine disconnection gracefully without crashing the Blender UI.
- Preserve Blender object materials, modifiers, and animation curves during mesh updates.

## Tests to run for changes here
- `python -m unittest tests.test_protocol`
- `python -m unittest tests.test_server_control`
- `python -m unittest tests.test_shm`

## Known gotchas and open TODOs
- Persistent instance overrides across recooks for Scene Objects ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L154)).
- Blender Object picker and waiting state verification ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L177)).
