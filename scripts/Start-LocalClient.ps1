$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$game = Join-Path (Split-Path -Parent $root) 'KnightOnline1453'
$localGame = Join-Path $root '.local/client-1453'
if (Test-Path -LiteralPath (Join-Path $localGame '.reko-local-client.json')) { $game = $localGame }
if (Get-Process KnightOnline -ErrorAction SilentlyContinue) { throw 'Close the running game before starting the local-server client.' }
$serverIni = Get-Content -LiteralPath (Join-Path $game 'Server.ini') -Raw
$serverAddress = [regex]::Match($serverIni, '(?mi)^IP0=([^\r\n]+)')
$serverIp = $null
if (-not $serverAddress.Success -or -not [Net.IPAddress]::TryParse($serverAddress.Groups[1].Value.Trim(), [ref]$serverIp) -or $serverIp.AddressFamily -ne [Net.Sockets.AddressFamily]::InterNetwork) {
    throw 'Set IP0 in the selected client Server.ini to the RE-KO server IPv4 address.'
}
foreach ($port in @(15100,15001)) {
    $connection = New-Object Net.Sockets.TcpClient
    try {
        $pending = $connection.BeginConnect($serverIp, $port, $null, $null)
        if (-not $pending.AsyncWaitHandle.WaitOne(3000)) { throw 'Connection timed out.' }
        $connection.EndConnect($pending)
    }
    catch { throw "Cannot reach RE-KO at $serverIp TCP $port. Check the server, VPN and host firewall. $($_.Exception.Message)" }
    finally { $connection.Dispose() }
}
$log = Join-Path $root ('logs/local-client-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')
New-Item -ItemType Directory -Path (Join-Path $root 'logs') -Force | Out-Null
$python = (Get-Command python -ErrorAction Stop).Source
$process = Start-Process -FilePath $python -WorkingDirectory $root -ArgumentList @(('"' + (Join-Path $PSScriptRoot 'local_client.py') + '"'),'--game-dir',('"' + $game + '"')) -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError ($log + '.error') -PassThru
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
