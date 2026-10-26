param([string]$AdvertiseIp, [switch]$SkipBuild, [string]$RustLog = 'info')

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$listeners = @(Get-NetTCPConnection -State Listen -LocalPort 15001,15100 -ErrorAction SilentlyContinue)
if ($listeners.Count -gt 0) {
    throw "Server ports already occupied by PID(s): $($listeners.OwningProcess -join ', '). Stop that server before starting a capture."
}
$advertisedAddress = $null
$AdvertiseIp = $AdvertiseIp.Trim()
if (-not [string]::IsNullOrWhiteSpace($AdvertiseIp) -and $AdvertiseIp -ine 'auto') {
    if ($AdvertiseIp -ieq 'localhost') { $AdvertiseIp = '127.0.0.1' }
    if ($AdvertiseIp -notmatch '^\d{1,3}(\.\d{1,3}){3}$' -or -not [System.Net.IPAddress]::TryParse($AdvertiseIp, [ref]$advertisedAddress)) {
        throw 'Advertised address must be an IPv4 host address, or auto.'
    }
    $addressBytes = $advertisedAddress.GetAddressBytes()
    if ($advertisedAddress.ToString() -eq '0.0.0.0' -or $advertisedAddress.ToString() -eq '255.255.255.255' -or ($addressBytes[0] -ge 224 -and $addressBytes[0] -le 239)) {
        throw 'Advertise a reachable host address, not a bind, broadcast or multicast address.'
    }
}
Push-Location $projectRoot
$previousRustLog = $env:RUST_LOG
$previousLogFile = $env:REKO_LOG_FILE
$previousAdvertiseIp = $env:REKO_ADVERTISE_IP
try {
    if (-not $SkipBuild) {
        cargo build -p reko-serverd
        if ($LASTEXITCODE -ne 0) { throw 'Server build failed.' }
    }
    python (Join-Path $PSScriptRoot 'database_manager.py') update
    if ($LASTEXITCODE -ne 0) { throw 'Database preparation failed; server was not started. See docs/setup.md.' }
    $logDir = Join-Path $projectRoot 'logs'
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
    $captureName = 'capture-' + (Get-Date -Format 'yyyyMMdd-HHmmss')
    $env:REKO_LOG_FILE = Join-Path $logDir "$captureName.log"
    $env:RUST_LOG = $RustLog
    $env:REKO_ADVERTISE_IP = if ($advertisedAddress) { $advertisedAddress.ToString() } else { $null }
    $serverProcess = Start-Process -FilePath (Join-Path $projectRoot 'target/debug/reko-serverd.exe') -WorkingDirectory $projectRoot -WindowStyle Hidden -RedirectStandardOutput (Join-Path $logDir "$captureName.stdout.log") -RedirectStandardError (Join-Path $logDir "$captureName.stderr.log") -PassThru
    $serverProcess.Id | Set-Content (Join-Path $logDir 'server.pid')
    $env:REKO_LOG_FILE | Set-Content (Join-Path $logDir 'latest-capture.txt')
    $readyDeadline = (Get-Date).AddSeconds(30)
    do {
        $serverProcess.Refresh()
        if ($serverProcess.HasExited) {
            Get-Content (Join-Path $logDir "$captureName.stderr.log")
            throw "Server exited with code $($serverProcess.ExitCode)."
        }
        $ready = @(Get-NetTCPConnection -State Listen -LocalPort 15001,15100 -ErrorAction SilentlyContinue | Where-Object OwningProcess -eq $serverProcess.Id)
        if ($ready.Count -eq 2) { break }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $readyDeadline)
    if ($ready.Count -ne 2) { throw 'Server is running but both listeners are not ready. Inspect the capture log.' }
    Write-Output "Server PID: $($serverProcess.Id)"
    Write-Output "Capture: $env:REKO_LOG_FILE"
    $addressMode = if ($advertisedAddress) { $advertisedAddress.ToString() } else { 'automatic (login connection destination)' }
    Write-Output "Advertised game address: $addressMode; game TCP 15001, login TCP 15100"
    Write-Output 'Ready for login and character creation. Use scripts/Watch-Server.ps1 to follow the capture.'
    $startupWatcher=Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'Watch-LocalClientStartup.ps1'),'-ServerPid',$serverProcess.Id) -WindowStyle Hidden -RedirectStandardOutput (Join-Path $logDir "$captureName.client-startup.log") -RedirectStandardError (Join-Path $logDir "$captureName.client-startup.error.log") -PassThru
    $startupWatcher.Id | Set-Content (Join-Path $logDir 'client-startup-watcher.pid')
}
finally {
    $env:RUST_LOG = $previousRustLog
    $env:REKO_LOG_FILE = $previousLogFile
    $env:REKO_ADVERTISE_IP = $previousAdvertiseIp
    Pop-Location
}
