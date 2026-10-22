# Template for a Bob Codex job: WT (worktree name), BRANCH, PROMPT (prompt file on Bob), OUT (log name). Writes JSON events to C:UsersGrahamOUT.log.
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $env:PATH
Set-Location C:\Users\Graham\minihou_bench\MiniHou
git fetch C:\Users\Graham\native_head23.bundle native-head23:native-head23 2>&1 | Out-Null
if (Test-Path C:\Users\Graham\minihou_bench\WT) { git worktree remove C:\Users\Graham\minihou_bench\WT --force 2>&1 | Out-Null }
git branch -D BRANCH 2>&1 | Out-Null
git worktree add -B BRANCH C:\Users\Graham\minihou_bench\WT native-head23 2>&1 | Out-Null
Set-Location C:\Users\Graham\minihou_bench\WT
Get-Content C:\Users\Graham\PROMPT -Raw | codex exec --json -m gpt-6-sol -c 'model_reasoning_effort="medium"' -C C:\Users\Graham\minihou_bench\WT --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check - 2>&1 | Out-File -Encoding utf8 C:\Users\Graham\OUT.log
'done' | Out-File -Append C:\Users\Graham\OUT.log
