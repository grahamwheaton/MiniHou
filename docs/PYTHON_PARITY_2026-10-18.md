# Python parity on this worktree (2026-10-18)

`minihou_rs` is built from `rust/crates/minihou-py` with Maturin. Use the repo
`.venv`, `CARGO_BUILD_JOBS=2`, `CARGO_TARGET_DIR=rust/target_pyparity`,
`CARGO_PROFILE_DEV_DEBUG=0`, and `CARGO_INCREMENTAL=0`; then run
`.venv/Scripts/python.exe -m maturin develop --manifest-path rust/crates/minihou-py/Cargo.toml`.
The extension imports from `.venv/lib/site-packages/minihou_rs` and exports
`match_size`. This was a debug build; no workspace Rust tests or release build
were run.

On this worktree before building, the two suites ran 37 tests: 15 passed and
22 Rust parity tests skipped because the venv had no extension. After building
but before source changes: 37 tests, 9 failures. Seven failure reports were
curve/circle `__viewport_curve` attribute differences (including downstream
delete), one was the obsolete exact 18-node registry expectation, and one was
the obsolete expectation that `wrangle` was wholly unported.
After the bridge and contract updates: 38 tests, 0 failures, 0 skips.

The Rust core attaches `__viewport_curve` to some native viewport curves.
Python geometry has no corresponding attribute, so the Python bridge drops
that native hint when converting a result. The original 18 parity nodes remain
required and their Python parameter defaults are still checked. The Rust
registry has since grown to 182 nodes, and its `curve` also has native-only
`split` and `as_mesh` controls; their defaults are checked explicitly.
`wrangle` now supports a subset, so the parity suite checks that multiple
assignments are refused rather than classing the entire node as unported.
No node in these two suites needs an expected skip; no tested geometry differs
after bridge conversion.
