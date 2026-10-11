@echo off
setlocal
pushd "%~dp0"
"%~dp0build\runtime\python312\python.exe" -m minihou serve "%~dp0examples\rbd_wall.json" --port 8875 --http-port 8876
popd
pause
