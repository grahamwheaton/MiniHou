## Purpose
Compilation recipes, test invocation commands, concurrency tuning, metadata generation, and test environment variables.

## Key files
- `rust/Cargo.toml`: Workspace configuration for all Rust crates (`minihou-core`, `minihou-app`, `minihou-py`, `minihou-view`).
- `tools/dump_rust_param_meta.py`: Script generating `param_meta.json` from the Python node registry.
- `tests/test_rust_param_meta.py`: Test ensuring `param_meta.json` matches Python registry definitions.
- `tools/launch_blender.py`: Test runner configuring Blender paths and launching smoke suites.

## Key functions or types
`meta`, `main`, `ParamMetaTests`, `find_blender`, `gui_test`

## Conventions to follow
- Restrict compilation concurrency with `CARGO_BUILD_JOBS=2` to avoid system resource starvation.
- Set dedicated target directories (e.g. `CARGO_TARGET_DIR=rust/target_<area>`) per worktree.
- Run targeted tests with filters (`cargo test -p <crate> -- <filter>`); avoid unconditional full workspace tests.
- Always regenerate parameter metadata with `py tools/dump_rust_param_meta.py` after changing node parameters.

## Tests to run for changes here
- `cargo test -p minihou-core -- --help`
- `python -m unittest tests.test_rust_param_meta`
- `python -m unittest tests.test_portability`

## Known gotchas and open TODOs
- Environment variables: `BLENDER_EXE` specifies Blender binary; `MINIHOU_LINK_SELFTEST` specifies native app binary.
- `MINIHOU_NO_RUST=1` disables Rust backend in Python tests.
- Stale `minihou_rs` extension wheels cause parity test failures ([TODO_IMAGE_NODES.md](TODO_IMAGE_NODES.md#L185)).
