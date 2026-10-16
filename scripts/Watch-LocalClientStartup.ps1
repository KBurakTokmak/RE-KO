param([Parameter(Mandatory=$true)][int]$ServerPid)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$game=Join-Path (Split-Path -Parent $root) 'KnightOnline1453/KnightOnline.exe'
$python=(Get-Command python -ErrorAction Stop).Source
$seen=@{}
Write-Output 'Monitoring new original-client launches; startup-only shutdown profile, no code/file writes.'
while ($true) {
    $server=Get-Process -Id $ServerPid -ErrorAction SilentlyContinue
    if (-not $server -or $server.Path -ne (Join-Path $root 'target/debug/reko-serverd.exe')) { break }
    foreach ($client in @(Get-Process KnightOnline -ErrorAction SilentlyContinue)) {
        if ($client.Path -ine $game -or $seen.ContainsKey($client.Id)) {continue}
        $seen[$client.Id]=$true
        $age=((Get-Date)-$client.StartTime).TotalSeconds
        if ($age -gt 25) {Write-Output "PID $($client.Id) already running; refusing late startup profile.";continue}
        $stamp=Get-Date -Format 'yyyyMMdd-HHmmss'
        $log=Join-Path $root "logs/auto-client-$stamp-$($client.Id).log"
        $helper=Start-Process -FilePath $python -ArgumentList @((Join-Path $PSScriptRoot 'local_client.py'),'--attach-new',$client.Id) -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError "$log.error" -PassThru
        Write-Output "Observed client PID $($client.Id); profile/exit observer PID $($helper.Id); $log"
    }
    Start-Sleep -Milliseconds 300
}
Write-Output 'Server stopped; client startup monitor finished.'
