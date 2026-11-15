#requires -Version 5.1
[CmdletBinding()]
param([ValidatePattern('^[a-f0-9]{40}$')][string]$Revision = 'bd35e243fa436f141f015cf221a6e19d0c34b7ca')
$ErrorActionPreference = 'Stop'
$path = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../local-data/materials.ru.json'))
$catalog = Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json
if ($catalog.revision -ne $Revision) { throw 'Catalog revision differs; update the base catalog first.' }
$baseUri = "https://gitlab.com/Dimbreath/animegamedata2/-/raw/$Revision"
$cacheDir = Join-Path $PSScriptRoot "../local-data/catalog-cache/$Revision"
New-Item -ItemType Directory -Path $cacheDir -Force | Out-Null
function Read-PinnedJson([string]$relative) {
    $cached = Join-Path $cacheDir ($relative -replace '/','_')
    if (!(Test-Path -LiteralPath $cached)) {
        # Each successful download survives rate limits or interrupted runs.
        $temporary = "$cached.tmp"
        Invoke-WebRequest -UseBasicParsing "$baseUri/$relative" -OutFile $temporary
        $null = Get-Content -LiteralPath $temporary -Raw -Encoding UTF8 | ConvertFrom-Json
        Move-Item -LiteralPath $temporary -Destination $cached -Force
    }
    Get-Content -LiteralPath $cached -Raw -Encoding UTF8 | ConvertFrom-Json
}
$texts = @{}
foreach ($prefix in 'TextMapRU','TextMap_MediumRU') {
    foreach ($part in 0,1) {
        Write-Host "Loading $prefix/$part..."
        $map = Read-PinnedJson "TextMap/${prefix}_$part.json"
        foreach ($property in $map.PSObject.Properties) { $texts[$property.Name] = [string]$property.Value }
    }
}
$definitions = @{}
foreach ($table in 'BydMaterialExcelConfigData','BeyondHallFacilityExcelConfigData','TpsWeaponExcelConfigData') {
    Write-Host "Loading $table..."
    $entries = Read-PinnedJson "ExcelBinOutput/$table.json"
    foreach ($entry in $entries) {
        $id = [string]$entry.id
        $name = $texts[[string]$entry.nameTextMapHash]
        if (!$name) { continue }
        $catalog.item_names | Add-Member -NotePropertyName $id -NotePropertyValue $name -Force
        if ($entry.rankLevel) { $catalog.rarities | Add-Member -NotePropertyName $id -NotePropertyValue ([int]$entry.rankLevel) -Force }
        $definitions[$id] = @{ item_type=$entry.itemType; source_table=$table; name_text_map_hash=$entry.nameTextMapHash; description=$texts[[string]$entry.descTextMapHash] }
    }
}
if (!$definitions.Count) { throw 'No extended items resolved; catalog was not changed.' }
$catalog | Add-Member -NotePropertyName extended_items -NotePropertyValue $definitions -Force
$temporary = "$path.extended.tmp"
[IO.File]::WriteAllText($temporary,($catalog | ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
Move-Item -LiteralPath $temporary -Destination $path -Force
Write-Host "Added $($definitions.Count) extended item definitions: $path"
