@echo off
setlocal
pushd "%~dp0" || exit /b 1
git pull --ff-only
if errorlevel 1 (
    echo Repository update failed. Server startup cancelled; resolve the Git error and retry.
    popd
    pause
    exit /b 1
)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\Start-ServerWatch.ps1" %*
set "server_exit_code=%errorlevel%"
popd
if not "%server_exit_code%"=="0" pause
exit /b %server_exit_code%
