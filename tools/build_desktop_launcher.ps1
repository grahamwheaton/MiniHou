$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
& $compiler /nologo /target:winexe /reference:System.Windows.Forms.dll "/win32icon:$projectRoot\minihou\assets\minihou.ico" "/out:$projectRoot\MiniHou.exe" "$PSScriptRoot\DesktopLauncher.cs"
if ($LASTEXITCODE -ne 0) { throw 'MiniHou launcher compilation failed' }
