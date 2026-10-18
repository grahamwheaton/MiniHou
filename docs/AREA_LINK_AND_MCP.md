## Purpose
Inter-process link server, binary streaming protocol, embedded HTTP REST API, and Model Context Protocol (MCP) server.

## Key files
- `rust/crates/minihou-app/src/blender_link.rs`: Blender discovery, addon packaging, process spawning, and link server.
- `rust/crates/minihou-app/src/link_protocol.rs`: Binary message framing, array encoding, and geometry synchronization.
- `rust/crates/minihou-app/src/link_instances.rs`: Shared memory and socket instance bridge management.
- `rust/crates/minihou-app/src/http_api.rs`: Embedded HTTP server, REST endpoints, and MCP command routing.
- `tools/setup_mcp.py`: AI assistant MCP registration utility configuring `.mcp.json`.

## Key functions or types
`LinkServer`, `LinkRequest`, `HttpServer`, `Request`, `Response`, `Array`, `InstanceManager`, `encode_message`, `read_message`, `find_installs`

## Conventions to follow
- Binary streaming protocol uses length-prefixed JSON headers followed by raw typed array buffers.
- HTTP server rejects cross-origin requests and blocks arbitrary Python execution without permission.
- Handle socket disconnection cleanly without leaving orphaned background engine instances.

## Tests to run for changes here
- `cargo test -p minihou-app -- http_tests`
- `cargo test -p minihou-app -- link_protocol`
- `python -m unittest tests.test_rust_link`
- `python -m unittest tests.test_rust_mcp_support`

## Known gotchas and open TODOs
- Fence and MCP integration test completion ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L202)).
- Archviz procedural asset generators via MCP when idle ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L210)).
