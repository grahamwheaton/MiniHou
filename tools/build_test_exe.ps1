# Build a MiniHou test exe quickly.
# 1. Uses one dedicated build worktree (rebuilt incrementally, not from scratch).
# 2. Uses one shared target folder for this build worktree only (parallel agents keep their own).
# 3. Builds the testbuild profile (release optimisation, no thin LTO, incremental).
# Run the full test suite separately before merging.
param(
  [string]$Build = "C:\Users\Grayw\Documents\Minihou_Rust\.claude\worktrees\build-test",
  [string]$Ref = "native-ui",
  [string]$Out = "C:\Users\Grayw\Documents\Minihou_Rust\MiniHou_test"
)
$repo = "C:\Users\Grayw\Documents\Minihou_Rust\.claude\worktrees\native-ui-cont"
$target = "C:\Users\Grayw\Documents\Minihou_Rust\target-testbuild"
if (-not (Test-Path $Build)) { git -C $repo worktree add --detach $Build $Ref | Out-Null }
# Build from the local ref: worktrees share this repo's refs, so no GitHub push or fetch is needed.
git -C $Build checkout -q --detach $Ref
$head = (git -C $Build rev-parse --short=9 HEAD)
$env:CARGO_TARGET_DIR = $target
# Use all cores for the build; the cap of 2 only slowed it down.
Remove-Item Env:CARGO_BUILD_JOBS -ErrorAction SilentlyContinue
Push-Location "$Build\rust"
cargo build -p minihou-app --profile testbuild
$code = $LASTEXITCODE
Pop-Location
if ($code -ne 0) { Write-Error "build failed"; exit $code }
Copy-Item "$target\testbuild\minihou-app.exe" "$Out\minihou-app_$head.exe" -Force
Write-Output "built $head -> $Out\minihou-app_$head.exe"
