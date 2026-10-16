$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$expectedExe = [IO.Path]::GetFullPath((Join-Path $projectRoot 'target/debug/reko-serverd.exe'))
$servers = @(Get-Process reko-serverd -ErrorAction SilentlyContinue | Where-Object { $_.Path -ieq $expectedExe })
foreach ($serverProcess in $servers) {
    Stop-Process -Id $serverProcess.Id -ErrorAction Stop
    if (-not $serverProcess.WaitForExit(10000)) { throw "Server PID $($serverProcess.Id) did not stop." }
    Write-Output "Stopped RE-KO server PID $($serverProcess.Id)."
}
if ($servers.Count -eq 0) { Write-Output 'RE-KO server is already stopped.' }

$watcherPath = Join-Path $PSScriptRoot 'Watch-LocalClientStartup.ps1'
$watchers = @(Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" | Where-Object {
    $_.CommandLine -and $_.CommandLine.IndexOf($watcherPath, [StringComparison]::OrdinalIgnoreCase) -ge 0
})
foreach ($watcher in $watchers) {
    Stop-Process -Id $watcher.ProcessId -ErrorAction SilentlyContinue
}

$database = Join-Path $projectRoot 'reko.db'
$python = Get-Command python -ErrorAction SilentlyContinue
if ((Test-Path -LiteralPath $database) -and $python) {
    & $python.Source -c "import sqlite3,sys; db=sqlite3.connect(sys.argv[1],timeout=5); result=db.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone(); db.close(); print('Save checkpoint:',result); sys.exit(1 if result[0] else 0)" $database
    if ($LASTEXITCODE -ne 0) { throw 'Server stopped, but database checkpoint failed; another tool may still have the save open.' }
} elseif (Test-Path -LiteralPath $database) {
    Write-Warning 'Python is unavailable. Preserve reko.db together with any reko.db-wal/reko.db-shm files when copying the save.'
}
Write-Output 'RE-KO and its startup watcher are stopped.'
