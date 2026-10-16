param([switch]$SkipBuild, [string]$RustLog = 'info')

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$listeners = @(Get-NetTCPConnection -State Listen -LocalPort 15001,15100 -ErrorAction SilentlyContinue)
if ($listeners.Count -gt 0) {
    throw "Server ports already occupied by PID(s): $($listeners.OwningProcess -join ', '). Stop that server before starting a capture."
}
Push-Location $projectRoot
$previousRustLog = $env:RUST_LOG
$previousLogFile = $env:REKO_LOG_FILE
try {
    if (-not $SkipBuild) {
        cargo build -p reko-serverd
        if ($LASTEXITCODE -ne 0) { throw 'Server build failed.' }
    }
    $logDir = Join-Path $projectRoot 'logs'
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
    $captureName = 'capture-' + (Get-Date -Format 'yyyyMMdd-HHmmss')
    $env:REKO_LOG_FILE = Join-Path $logDir "$captureName.log"
    $env:RUST_LOG = $RustLog
    $serverProcess = Start-Process -FilePath (Join-Path $projectRoot 'target/debug/reko-serverd.exe') -WorkingDirectory $projectRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $logDir "$captureName.stdout.log") -RedirectStandardError (Join-Path $logDir "$captureName.stderr.log") -PassThru
    $serverProcess.Id | Set-Content (Join-Path $logDir 'server.pid')
    $env:REKO_LOG_FILE | Set-Content (Join-Path $logDir 'latest-capture.txt')
    Start-Sleep -Seconds 2
    $serverProcess.Refresh()
    if ($serverProcess.HasExited) {
        Get-Content (Join-Path $logDir "$captureName.stderr.log")
        throw "Server exited with code $($serverProcess.ExitCode)."
    }
    $ready = @(Get-NetTCPConnection -State Listen -LocalPort 15001,15100 -ErrorAction SilentlyContinue | Where-Object OwningProcess -eq $serverProcess.Id)
    if ($ready.Count -ne 2) { throw 'Server is running but both listeners are not ready. Inspect the capture log.' }
    Write-Output "Server PID: $($serverProcess.Id)"
    Write-Output "Capture: $env:REKO_LOG_FILE"
    Write-Output 'Ready for login and character creation. Use scripts/Watch-Server.ps1 to follow the capture.'
    $startupWatcher=Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'Watch-LocalClientStartup.ps1'),'-ServerPid',$serverProcess.Id) -WindowStyle Hidden -RedirectStandardOutput (Join-Path $logDir "$captureName.client-startup.log") -RedirectStandardError (Join-Path $logDir "$captureName.client-startup.error.log") -PassThru
    $startupWatcher.Id | Set-Content (Join-Path $logDir 'client-startup-watcher.pid')
}
finally {
    $env:RUST_LOG = $previousRustLog
    $env:REKO_LOG_FILE = $previousLogFile
    Pop-Location
}
