#requires -Version 7.0
param([string]$Revision = 'bd35e243fa436f141f015cf221a6e19d0c34b7ca')
$ErrorActionPreference = 'Stop'
if ($Revision -notmatch '^[a-f0-9]{40}$') { throw 'Revision must be a full Git commit hash' }
$catalogDir = Join-Path $PSScriptRoot '..\local-data'
New-Item -ItemType Directory -Path $catalogDir -Force | Out-Null
$baseUri = "https://gitlab.com/Dimbreath/animegamedata2/-/raw/$Revision"
Write-Host 'Downloading material definitions and Russian names...'
$definitions = Invoke-RestMethod "$baseUri/ExcelBinOutput/MaterialExcelConfigData.json"
$texts = @{}
foreach ($prefix in @('TextMapRU','TextMap_MediumRU')) {
    foreach ($part in 0,1) {
        # These are complementary text pools: furniture and gear use different pools.
        $response = Invoke-WebRequest "$baseUri/TextMap/${prefix}_$part.json"
        $map = $response.Content | ConvertFrom-Json -AsHashtable
        foreach ($key in $map.Keys) { $texts[$key] = $map[$key] }
    }
}
$names = @{}
$inventoryDefinitions = @{}
$itemPresentation = @{}
function Get-ItemPresentation($entry) {
    @{icon=$(if ($entry.itemIcon) {$entry.itemIcon} else {$entry.icon}); rank=$entry.rank; rarity=$entry.rankLevel;
      typeLabel=$texts[[string]$entry.typeDescTextMapHash]; weaponType=$entry.weaponType; equipType=$entry.equipType; setId=$entry.setId}
}
foreach ($entry in $definitions) {
    $itemPresentation[[string]$entry.id] = Get-ItemPresentation $entry
    $inventoryDefinitions[[string]$entry.id] = @{itemType=$entry.itemType; materialType=$entry.materialType; systemType=$entry.JDDNMAMMEPO; rankLevel=$entry.rankLevel}
    $name = $texts[[string]$entry.nameTextMapHash]
    if ($name) { $names[[string]$entry.id] = $name }
}
if ($names.Count -eq 0) { throw 'No material names resolved; existing catalog was not changed' }
$itemNames = @{}
$rarities = @{}
foreach ($tableName in @('Weapon','Reliquary','HomeWorldFurniture','BydMaterial','BeyondHallFacility','TpsWeapon')) {
    Write-Host "Downloading $tableName definitions..."
    $itemDefinitions = Invoke-RestMethod "$baseUri/ExcelBinOutput/${tableName}ExcelConfigData.json"
    foreach ($entry in $itemDefinitions) {
        $itemPresentation[[string]$entry.id] = Get-ItemPresentation $entry
        $name = $texts[[string]$entry.nameTextMapHash]
        if ($name) { $itemNames[[string]$entry.id] = $name }
        if ($entry.rankLevel) { $rarities[[string]$entry.id] = [int]$entry.rankLevel }
    }
}
$catalog = @{ schema_version = 1; language = 'ru'; source = 'https://gitlab.com/Dimbreath/animegamedata2'; revision = $Revision; fetched_at = [DateTime]::UtcNow.ToString('o'); names = $names; item_names = $itemNames; rarities = $rarities; inventory_definitions=$inventoryDefinitions; item_presentation=$itemPresentation }
$catalogPath = Join-Path $catalogDir 'materials.ru.json'
$catalog | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $catalogPath -Encoding utf8
Write-Host "Saved $($names.Count) material names and $($itemNames.Count) item names to $catalogPath"
