$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$game = Join-Path (Split-Path -Parent $root) 'KnightOnline1453'
if (Get-Process KnightOnline -ErrorAction SilentlyContinue) { throw 'Close the running game before starting the local-server client.' }
if (-not (Get-NetTCPConnection -LocalPort 15001 -State Listen -ErrorAction SilentlyContinue)) { throw 'Start RE-KO server before the client.' }
$serverIni = Get-Content (Join-Path $game 'Server.ini')
if (-not ($serverIni | Where-Object { $_ -match '^IP0=127\.0\.0\.1\s*$' })) { throw 'This startup profile requires the local RE-KO server configuration.' }
$log = Join-Path $root ('logs/local-client-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')
New-Item -ItemType Directory -Path (Join-Path $root 'logs') -Force | Out-Null
$python = (Get-Command python -ErrorAction Stop).Source
$process = Start-Process -FilePath $python -WorkingDirectory $root -ArgumentList @((Join-Path $PSScriptRoot 'local_client.py')) -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError ($log + '.error') -PassThru
$log | Set-Content (Join-Path $root 'logs/latest-local-client.txt')
Write-Output "Local client startup PID $($process.Id); log $log"
$deadline = (Get-Date).AddSeconds(50)
while ((Get-Date) -lt $deadline) {
    $content = Get-Content -LiteralPath $log -Raw -ErrorAction SilentlyContinue
    if ($content -match 'LOCAL_PROFILE_ALREADY_MANAGED:') {
        Write-Output 'The server startup monitor already owns this client profile. Check logs/auto-client-* for readiness and exit status.'
        return
    }
    if ($content -match 'LOCAL_PROFILE_READY:') {
        Write-Output 'Local client ready; the legacy heartbeat watchdog completed normal shutdown.'
        return
    }
    $process.Refresh()
    if ($process.HasExited) {
        $details = Get-Content -LiteralPath ($log + '.error') -Raw -ErrorAction SilentlyContinue
        throw "Local client startup failed. See $log. $details"
    }
    Start-Sleep -Milliseconds 200
}
throw "Local client startup did not confirm readiness. See $log before retrying."
