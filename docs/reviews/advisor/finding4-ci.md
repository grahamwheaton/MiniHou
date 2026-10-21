ive-ui-cont/.github/workflows/ci.yml#L3-L7), `on.push.branches` only lists `[main, master, "ccr-*", "Rust"]`. Pushes to `native-ui` (or `native-ui-*`) do not trigger CI unless a PR is opened.
- **Default members exclude `minihou-app`:** In [Cargo.toml](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/Cargo.toml#L3-L4), `members` includes `"crates/minihou-view"` and `"crates/minihou-app"`, while `default-members` is restricted to `["crates/minihou-core", "crates/minihou-py"]`.
- **CI commands omit `--workspace`:** In [ci.yml](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/.github/workflows/ci.yml#L35-L41), the `rust` job runs `cargo test --release` (line 37) and `cargo clippy --all-targets -- -D warnings` (line 41) without `--workspace` or `-p minihou-app`. Consequently, even on PRs or `main`, `minihou-app` (and `minihou-view`) are never compiled, linted, or unit-tested in CI.

**2. Real Risk & Severity**

**High.** Active development on `native-ui` has zero automated compilation, Clippy, or test gating. Additionally, changes to `minihou-core` on any branch can silently break `minihou-app` compilation or tests without CI failing, because `minihou-app` is excluded from default workspace commands even when CI runs.

**3. Smallest Safe Fix**

1. In [ci.yml](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/.github/workflows/ci.yml#L5), add `"native-ui*"` to `on.push.branches`.
2. In [ci.yml](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/.github/workflows/ci.yml#L35-L41), pass `--workspace` (or `-p minihou-app`) to `cargo test` and `cargo clippy` (installing `libgtk-3-dev` on Ubuntu if required by [minihou-app/Cargo.toml](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/Cargo.toml#L19) `rfd` / Wayland / X11 dependencies), or add `crates/minihou-app` to `default-members` in [rust/Cargo.toml](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/Cargo.toml#L4).

**4. Test That Would Prove It**

Run `cargo test --release` vs. `cargo test --workspace --release` in `rust/` and verify that `minihou-app` tests only execute in the latter; then trigger the updated workflow on a `native-ui` push with a deliberate lint/test failure in `crates/minihou-app` and confirm the GitHub Actions job triggers and fails.

**5. What Could Not Be Verified**

- **GitHub Actions run history:** The review's claim that *"No GitHub Actions runs were found for the `native-ui` branch"* depends on remote GitHub Actions state, which cannot be verified from local repository files alone.
- **Headless GPU / Linux system packages on CI runners:** Whether `minihou-app` tests require headless Vulkan/DX12 adapters or extra apt packages (`libgtk-3-dev`, `libxcb-*`, `libwayland-dev`) on GitHub's `ubuntu-latest` / `windows-latest` runners was not verified in a live runner environment.

