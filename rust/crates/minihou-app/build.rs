//! Embeds a build identifier (package version, git commit, build time) shown in the status bar.
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// UTC "YYYY-MM-DD HH:MM" from a unix timestamp (civil-from-days, no time-zone data needed).
fn utc_stamp(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3_600, rem % 3_600 / 60)
}

fn main() {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let hash = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "nogit".into());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    println!("cargo:rustc-env=MINIHOU_BUILD_COMMIT={hash}{}", if dirty { "*" } else { "" });
    println!("cargo:rustc-env=MINIHOU_BUILD_TIME={} UTC", utc_stamp(secs));
    // Any source change in the app or core re-runs this script so the build time stays honest.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../minihou-core/src");
    println!("cargo:rerun-if-changed=build.rs");
}
