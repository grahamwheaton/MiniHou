# MiniHou: source of truth

- **Repo:** `https://github.com/grahamwheaton/MiniHou.git`, branch `main`.
- **Canonical local checkout:** `C:\Users\Grayw\orca_MiniHou\MiniHou`. Start all work here.
- **Shipping app:** the Rust native app, `rust/crates/minihou-app`.
  - Build: `cargo build --release --manifest-path rust/Cargo.toml -p minihou-app`
  - Output: `rust/target/release/minihou-app.exe`, copied to `dist/rust-app/minihou-app.exe`.
- **Not a shipping target:** the PyInstaller packaging (`tools/build_app.py`). Don't build or ship it.

## Do not use as a base

- Other local copies: `C:\Users\Grayw\Documents\MiniHou-GemHome`, `C:\Users\Grayw\Documents\Minihou-OC\MiniHou`, `C:\Users\Grayw\Documents\Minihou_Rust`. They are older or on other feature branches.
- Remote branches other than `main` (for example `Rust`, `codex/*`, `claude/*`, `agy/*`, `feature/*`, `ccr-*`). Their tips are kept as `archive/*` tags. Don't branch from them, and don't merge them into `main` without the user's approval.

## Working rules

- Branch from `main` for each change, then merge back to `main`.
- Don't push to `origin` or force-push without asking the user first.
- Build artifacts (`dist/`, `build/`, `rust/target/`) are gitignored. Don't commit them.
