param([string]$SourceDirectory, [string]$ServerIp)

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
if ([string]::IsNullOrWhiteSpace($ServerIp)) {
    $sourceSettings = Get-Content -LiteralPath (Join-Path $source 'Server.ini') -Raw
    $sourceAddress = [regex]::Match($sourceSettings, '(?mi)^IP0=([^\r\n]+)')
    $ServerIp = if ($sourceAddress.Success) { $sourceAddress.Groups[1].Value.Trim() } else { '127.0.0.1' }
}
$ServerIp = $ServerIp.Trim()
$parsedAddress = $null
if ($ServerIp -notmatch '^\d{1,3}(\.\d{1,3}){3}$' -or -not [Net.IPAddress]::TryParse($ServerIp, [ref]$parsedAddress)) {
    throw 'ServerIp/IP0 must be the RE-KO server IPv4 address.'
}
$ServerIp = $parsedAddress.ToString()

Push-Location $root
try {
    cargo build -p reko-tools --bin reko-local-guard --bin XPatch
    if ($LASTEXITCODE -ne 0) { throw 'Local client helper build failed.' }
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination -Recurse
    Copy-Item -LiteralPath (Join-Path $root 'target/debug/reko-local-guard.exe') -Destination (Join-Path $destination 'XTrap/XTrap.exe') -Force
    Copy-Item -LiteralPath (Join-Path $root 'target/debug/XPatch.exe') -Destination (Join-Path $destination 'XPatch.exe') -Force
    [IO.File]::WriteAllText((Join-Path $destination 'Server.ini'), "[Server]`r`nCount=1`r`nIP0=$ServerIp`r`n[Version]`r`nFiles=1453`r`n")
    @{
        profile = 'local-1453-inert-guard-v1'
        client_sha256 = $expectedHash
        guard_sha256 = (Get-FileHash -LiteralPath (Join-Path $destination 'XTrap/XTrap.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
        patch_sha256 = (Get-FileHash -LiteralPath (Join-Path $destination 'XPatch.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
    } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination '.reko-local-client.json') -Encoding ASCII
    Write-Output "Local client prepared: $destination"
    python (Join-Path $PSScriptRoot 'local_client_tables.py')
    if ($LASTEXITCODE -ne 0) { throw 'Local English transformation table preparation failed.' }
    Write-Output 'Start it with scripts/Start-LocalClient.cmd. Original source files were preserved.'
}
finally { Pop-Location }
