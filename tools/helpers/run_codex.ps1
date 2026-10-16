# Run a Codex (Sol) worker from a prompt file:
#   powershell -File tools/helpers/run_codex.ps1 <prompt.txt> <out.txt> [medium|high]
param($task, $out, $effort = "medium")
Get-Content $task -Raw | codex exec -m gpt-6.1-sol -c model_reasoning_effort=$effort --dangerously-bypass-approvals-and-sandbox - *> $out
