param(
    [Parameter(Mandatory=$true)][string]$Server,
    [Parameter(Mandatory=$true)][string]$Database,
    [string]$OutputDirectory = (Join-Path (Split-Path -Parent $PSScriptRoot) 'logs/reference1453')
)
$ErrorActionPreference = 'Stop'
$builder = New-Object System.Data.SqlClient.SqlConnectionStringBuilder
$builder['Data Source'] = $Server
$builder['Initial Catalog'] = $Database
$builder['Integrated Security'] = $true
$builder['TrustServerCertificate'] = $true
$connection = New-Object System.Data.SqlClient.SqlConnection $builder.ConnectionString
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
try {
    $connection.Open()
    $queries = [ordered]@{
        MAGIC_TYPE8 = 'SELECT * FROM MAGIC_TYPE8 ORDER BY iNum'
        MAGIC_TYPE6 = 'SELECT * FROM MAGIC_TYPE6 WHERE iNum / 1000 = 472 ORDER BY iNum'
        MAGIC_TYPE6_NPC = 'SELECT * FROM MAGIC_TYPE6 WHERE iNum BETWEEN 500505 AND 500512 ORDER BY iNum'
        ZONE_INFO = 'SELECT * FROM ZONE_INFO ORDER BY ZoneNo'
    }
    foreach ($entry in $queries.GetEnumerator()) {
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
        $path = Join-Path $OutputDirectory ($entry.Key + '.json')
        [System.IO.File]::WriteAllText($path, (ConvertTo-Json -InputObject $rows -Depth 5), (New-Object System.Text.UTF8Encoding $false))
        Write-Output "$($entry.Key): $($rows.Count) rows"
    }
}
finally { $connection.Dispose() }
