param(
    [Parameter(Mandatory=$true)][string]$Server,
    [Parameter(Mandatory=$true)][string]$Database,
    [string]$OutputDirectory = '',
    # Optional supplementary 2086 database for accessory compound recipes (absent from 1453 data).
    [string]$SupplementaryDatabase = '',
    [string]$SupplementaryOutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repository 'logs/reference1453' }
if (-not $SupplementaryOutputDirectory) { $SupplementaryOutputDirectory = Join-Path $repository 'logs/reference2086' }
function Export-Queries([string]$DatabaseName, [string]$Directory, $Queries) {
    $builder = New-Object System.Data.SqlClient.SqlConnectionStringBuilder
    $builder['Data Source'] = $Server
    $builder['Initial Catalog'] = $DatabaseName
    $builder['Integrated Security'] = $true
    $builder['TrustServerCertificate'] = $true
    $connection = New-Object System.Data.SqlClient.SqlConnection $builder.ConnectionString
    New-Item -ItemType Directory -Path $Directory -Force | Out-Null
    try {
        $connection.Open()
        foreach ($entry in $Queries.GetEnumerator()) {
            $command = $connection.CreateCommand()
            $command.CommandText = $entry.Value
            $reader = $command.ExecuteReader()
            $rows = @(
                while ($reader.Read()) {
                    $row = [ordered]@{}
                    for ($index=0; $index -lt $reader.FieldCount; $index++) {
                        $row[$reader.GetName($index)] = if ($reader.IsDBNull($index)) { $null } else { $reader.GetValue($index) }
                    }
                    [pscustomobject]$row
                }
            )
            $reader.Close()
            $path = Join-Path $Directory ($entry.Key + '.json')
            [System.IO.File]::WriteAllText($path, (ConvertTo-Json -InputObject $rows -Depth 5), (New-Object System.Text.UTF8Encoding $false))
            Write-Output "$($entry.Key): $($rows.Count) rows"
        }
    }
    finally { $connection.Dispose() }
}
Export-Queries $Database $OutputDirectory ([ordered]@{
    MAGIC_TYPE1 = 'SELECT * FROM MAGIC_TYPE1 ORDER BY iNum'
    MONSTER_MAGIC = 'SELECT sSid, byMagicAttack FROM K_MONSTER ORDER BY sSid'
    MAGIC_TYPE8 = 'SELECT * FROM MAGIC_TYPE8 ORDER BY iNum'
    MAGIC_TYPE6 = 'SELECT * FROM MAGIC_TYPE6 WHERE iNum / 1000 = 472 ORDER BY iNum'
    MAGIC_TYPE6_NPC = 'SELECT * FROM MAGIC_TYPE6 WHERE iNum BETWEEN 500505 AND 500512 ORDER BY iNum'
    ZONE_INFO = 'SELECT * FROM ZONE_INFO ORDER BY ZoneNo'
    MONSTER_CHALLENGE_SUMMON_LIST = 'SELECT sIndex, bLevel, bStage, bStageLevel, sTime, sSid, sCount, sPosX, sPosZ, bRange FROM MONSTER_CHALLENGE_SUMMON_LIST ORDER BY sIndex'
})
if ($SupplementaryDatabase) {
    Export-Queries $SupplementaryDatabase $SupplementaryOutputDirectory ([ordered]@{
        ACCESSORY_UPGRADE = 'SELECT nIndex, strNote, nOriginType, nOriginItem, nReqItem1, nReqNoah, nGiveItem, nNPCNum FROM ITEM_UPGRADE WHERE bRateType = 8 ORDER BY nIndex'
    })
}
