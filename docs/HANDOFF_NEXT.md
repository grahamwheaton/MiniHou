# Where we are (end of day, 15 Oct 2026) and how to carry on

Everything from today is on the GitHub branch **`native-ui`** (repo `grahamwheaton/MiniHou`).

## Get it on another PC
```
git fetch origin
git checkout native-ui          # or: git switch native-ui
git pull origin native-ui
```
(Use a fresh clone if the folder has local changes you do not want to mix: `git clone -b native-ui https://github.com/grahamwheaton/MiniHou.git`.)

## Build and run the native editor
Needs Rust 1.99 (the repo pins it in `rust-toolchain.toml`; `rustup` installs it on first use).
```
cargo build --release --manifest-path rust/Cargo.toml -p minihou-app
rust\target\release\minihou-app.exe                 # opens the built-in demo graph
rust\target\release\minihou-app.exe path\to\graph.json
```
Run the checks: `cargo test --release --manifest-path rust/Cargo.toml` and
`cargo test --release --manifest-path rust/Cargo.toml -p minihou-app`, then
`cargo clippy --release --manifest-path rust/Cargo.toml --workspace --all-targets -- -D warnings`.
The Python parity tests need a wheel built into a venv (see `rust/PORTING.md`).

## What is in `native-ui`
- The native editor (egui + wgpu): shelf, network editor, parameter panel, viewport with header, tool strip, timeline,
  geometry info panel, Blender link bar, preferences, units (mm), keymap (Blender parity) with audit in `docs/KEYMAP_AUDIT.md`,
  user guide in `rust/crates/minihou-app/docs/NATIVE_EDITOR.md`.
- Viewport tools: transform handles, modal G / R / S with axis locks, snapping, two-point rotate, scale cage; Shift+R repeat;
  sketch and tape tools; primitive handles; selection modes; silhouette outline; 3D cursor (Shift+RMB); drop placement.
- Rust core nodes ported (bit-exact with Python): grid, box, point, curve, line, circle, sphere, tube, torus, bound,
  match_size, delete, transform (group + proportional), normal, noise, merge, plus `lod_bake` (clustered LOD cache).
- Design source for the UI is in `rust/crates/minihou-app/design/` (the Claude Design canvas files).

## Known gaps and next steps
1. Not clicked through in a live window: modal operators (B snap, two-point rotate, C cage), primitive handles, drop preview.
   Please try them and note what feels wrong.
2. Nodes not in the Rust core yet cannot cook in the native app. Idea discussed: a Python fallback evaluator so every
   shelf node works (slower), then port hot nodes by priority. Not started.
3. `lod_bake` viewport mode ("Clustered LOD" in the proxy menu) needs real-mesh timing and tuning; see `docs/NANITE_PROXY.md`.
4. Pre-Rust features still missing are listed in `docs/KEYMAP_AUDIT.md` (network boxes, find node, paint, knife, loop cut, pie
   menu radial interaction, component deletion).
5. Face picking works per triangle (a quad shows as two triangles).
6. Blender link: the native app cannot yet be connected to by Blender (no link server in the native app).
