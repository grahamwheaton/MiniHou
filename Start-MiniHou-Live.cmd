@echo off
cd /d "%~dp0"
"%~dp0build\runtime\python312\python.exe" -m minihou app --workspace "%~dp0." %*
if errorlevel 1 pause
