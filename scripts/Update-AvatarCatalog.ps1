#requires -Version 5.1
[CmdletBinding()]
param([ValidatePattern('^[a-f0-9]{40}$')][string]$Revision = 'bd35e243fa436f141f015cf221a6e19d0c34b7ca')
$ErrorActionPreference = 'Stop'
$baseUri = "https://gitlab.com/Dimbreath/animegamedata2/-/raw/$Revision"
$maps = @{}
foreach ($prefix in 'TextMapRU','TextMap_MediumRU') {
    foreach ($part in 0,1) {
        Write-Host "Loading $prefix/$part..."
        $map = Invoke-RestMethod "$baseUri/TextMap/${prefix}_$part.json"
        foreach ($property in $map.PSObject.Properties) { $maps[$property.Name] = [string]$property.Value }
    }
}
$avatars = Invoke-RestMethod "$baseUri/ExcelBinOutput/AvatarExcelConfigData.json"
$depots = Invoke-RestMethod "$baseUri/ExcelBinOutput/AvatarSkillDepotExcelConfigData.json"
$skills = Invoke-RestMethod "$baseUri/ExcelBinOutput/AvatarSkillExcelConfigData.json"
$names = @{}
foreach ($avatar in $avatars) {
    $name = $maps[[string]$avatar.nameTextMapHash]
    if ($name) { $names[[string]$avatar.id] = $name }
}
$skillNames = @{}
foreach ($skill in $skills) {
    $name = $maps[[string]$skill.nameTextMapHash]
    if ($name) { $skillNames[[string]$skill.id] = $name }
}
if ($names.Count -eq 0) { throw 'No avatar names resolved.' }
$catalog = @{ schema_version=1; language='ru'; source='https://gitlab.com/Dimbreath/animegamedata2'; revision=$Revision; names=$names; skill_names=$skillNames; avatars=[object[]]@($avatars | ForEach-Object {$_}); skill_depots=[object[]]@($depots | ForEach-Object {$_}); skills=[object[]]@($skills | ForEach-Object {$_}) }
$path = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../local-data/avatars.ru.json'))
[IO.File]::WriteAllText($path,($catalog | ConvertTo-Json -Depth 30),[Text.UTF8Encoding]::new($false))
Write-Host "Saved $($names.Count) avatar names and $($skillNames.Count) skill names: $path"
