param([int]$WaitMinutes = 30, [int]$AttachDelaySeconds = 150, [string]$DebuggerPath = $env:REKO_CDB)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$clientDirectory = if ($env:REKO_GAME_DIR) { $env:REKO_GAME_DIR } else { Join-Path (Split-Path -Parent $root) 'KnightOnline1453' }
$clientPath = Join-Path $clientDirectory 'KnightOnline.exe'
if (-not $DebuggerPath) { $DebuggerPath = (Get-Command cdb.exe -ErrorAction Stop).Source }
$debugger = (Resolve-Path -LiteralPath $DebuggerPath).Path
$deadline = (Get-Date).AddMinutes($WaitMinutes)
while ((Get-Date) -lt $deadline) {
    $client = Get-Process KnightOnline -ErrorAction SilentlyContinue | Where-Object { $_.Path -ieq $clientPath } | Select-Object -First 1
    if ($client) { break }
    Start-Sleep -Seconds 1
}
if (-not $client) { throw 'No original client appeared before the trace deadline.' }
while (((Get-Date) - $client.StartTime).TotalSeconds -lt $AttachDelaySeconds) {
    Start-Sleep -Seconds 1
    $client.Refresh()
    if ($client.HasExited) { throw 'Client exited before delayed attachment.' }
}
$log = Join-Path $root ('logs/client-termination-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')
$commands = Join-Path $root 'scripts/trace-client-exit.txt'
$arguments = @('-p', $client.Id, '-G', '-logo', $log, '-cf', $commands)
$trace = Start-Process -FilePath $debugger -ArgumentList $arguments -WindowStyle Hidden -PassThru
$trace.Id | Set-Content (Join-Path $root 'logs/client-debugger.pid')
$log | Set-Content (Join-Path $root 'logs/latest-client-termination.txt')
Write-Output "Attached client PID $($client.Id), debugger PID $($trace.Id), log $log"
