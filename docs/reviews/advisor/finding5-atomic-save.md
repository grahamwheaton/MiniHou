/C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/action_dispatch.rs#L411-L422)), and HTTP MCP auto-save after `wait=1` edits ([http_ui.rs](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/http_ui.rs#L216)).
- Notably, [`Preferences::save_to`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/ui/prefs.rs#L59-L65) already uses a temp-file + `fs::rename` pattern, whereas [`Document::save`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs#L22) does not.

### (2) Real Risk & Severity: **Medium (High impact, Low probability)**

- `std::fs::write` opens the target path with `O_TRUNC` / `CREATE_ALWAYS` before writing bytes and does not call `sync_all()`.
- **Mitigating factor:** [`Document`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs#L10) already holds `self.text` serialized in memory, so serialization cannot fail mid-write, and typical graph JSONs are small (kilobytes).
- **Remaining hazard:** Disk-full errors, dropped network/cloud-synced drives, process crashes during frequent HTTP `/api/edit` auto-saves, or OS/power loss after truncation or before page-cache flush will leave a 0-byte or half-written `.json`/`.minihou` file with no backup.

### (3) Smallest Safe Fix

In [`Document::save`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs#L22), write to a sibling temporary file in `path.parent()`, flush/sync it, and atomically rename it over `path`:

```rust
pub fn save(&self, path: &Path) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("minihou.{}.tmp", std::process::id()));
    let mut f = std::fs::File::create(&tmp)?;
    if let Err(e) = f.write_all(self.text.as_bytes()).and_then(|()| f.sync_all()).and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}
```

### (4) Test That Would Prove It

Add a unit test in [document.rs](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/document.rs) (or a fault-injected save helper taking a writer/renamer) verifying:
1. Saving over an existing valid document replaces its contents cleanly and leaves no `.tmp` sibling behind.
2. When the write or rename step fails (e.g., simulating a write error before rename, or target path made non-replaceable), the original file at `path` remains byte-for-byte intact and the `.tmp` file is cleaned up.

### (5) What Could Not Be Verified

- Real-world filesystem/OS failure behavior (e.g., Windows `MoveFileExW` locks when external watchers/cloud-sync clients hold the target file open, or power-loss durability without directory `fsync`).

