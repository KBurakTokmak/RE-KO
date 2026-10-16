param([string]$LogPath)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $LogPath) {
    $LogPath = Get-Content (Join-Path $projectRoot 'logs/latest-capture.txt') -Raw
    $LogPath = $LogPath.Trim()
}
Write-Output "Following $LogPath (Ctrl+C stops this viewer; server keeps running)."
Get-Content -LiteralPath $LogPath -Tail 60 -Wait
