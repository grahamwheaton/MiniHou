param($model,$task,$out)
$p = (Get-Content $task -Raw) -replace '"', "'"
agy --model $model --dangerously-skip-permissions "-p=$p" *> $out
