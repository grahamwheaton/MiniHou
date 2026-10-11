@echo off
setlocal
pushd "%~dp0"
"%~dp0build\runtime\python312\python.exe" -m minihou serve "%~dp0examples\modifiers\transform.json" --port 8765 --http-port 8766
popd
pause
