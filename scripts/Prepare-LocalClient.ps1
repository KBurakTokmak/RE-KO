param([string]$SourceDirectory)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $SourceDirectory) { $SourceDirectory = Join-Path (Split-Path -Parent $root) 'KnightOnline1453' }
$source = (Resolve-Path -LiteralPath $SourceDirectory).Path
$destination = Join-Path $root '.local/client-1453'
$expectedHash = '9513ecdf29e99bfee307fa8dc93ef64bf616d77d475d57fde906107f32169bb0'
if ((Get-FileHash -LiteralPath (Join-Path $source 'KnightOnline.exe') -Algorithm SHA256).Hash -ine $expectedHash) {
    throw 'Unknown source client build.'
}
if (Test-Path -LiteralPath $destination) {
    throw 'Local client already exists. Preserve it before preparing a new copy.'
}

Push-Location $root
try {
    cargo build -p reko-tools --bin reko-local-guard --bin XPatch
    if ($LASTEXITCODE -ne 0) { throw 'Local client helper build failed.' }
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination -Recurse
    Copy-Item -LiteralPath (Join-Path $root 'target/debug/reko-local-guard.exe') -Destination (Join-Path $destination 'XTrap/XTrap.exe') -Force
    Copy-Item -LiteralPath (Join-Path $root 'target/debug/XPatch.exe') -Destination (Join-Path $destination 'XPatch.exe') -Force
    [IO.File]::WriteAllText((Join-Path $destination 'Server.ini'), "[Server]`r`nCount=1`r`nIP0=127.0.0.1`r`n[Version]`r`nFiles=1453`r`n")
    @{
        profile = 'local-1453-inert-guard-v1'
        client_sha256 = $expectedHash
        guard_sha256 = (Get-FileHash -LiteralPath (Join-Path $destination 'XTrap/XTrap.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
        patch_sha256 = (Get-FileHash -LiteralPath (Join-Path $destination 'XPatch.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
    } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination '.reko-local-client.json') -Encoding ASCII
    Write-Output "Local client prepared: $destination"
    Write-Output 'Start it with scripts/Start-LocalClient.cmd. Original source files were preserved.'
}
finally { Pop-Location }
