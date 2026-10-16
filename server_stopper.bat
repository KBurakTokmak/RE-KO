@echo off
setlocal
pushd "%~dp0" || exit /b 1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\Stop-Server.ps1"
set "server_exit_code=%errorlevel%"
popd
if not "%server_exit_code%"=="0" pause
exit /b %server_exit_code%
