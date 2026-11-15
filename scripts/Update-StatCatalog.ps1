#requires -Version 5.1
[CmdletBinding()]
param([ValidatePattern('^[a-f0-9]{40}$')][string]$Revision = 'bd35e243fa436f141f015cf221a6e19d0c34b7ca')
$ErrorActionPreference = 'Stop'
$catalog = @{schema_version=1; revision=$Revision; source='https://gitlab.com/Dimbreath/animegamedata2'}
$baseUri = "$($catalog.source)/-/raw/$Revision/ExcelBinOutput"
foreach ($entry in @(@('main_props','ReliquaryMainPropExcelConfigData'),@('affixes','ReliquaryAffixExcelConfigData'),@('levels','ReliquaryLevelExcelConfigData'))) {
    $data = Invoke-RestMethod "$baseUri/$($entry[1]).json"
    $catalog[$entry[0]] = [object[]]@($data | ForEach-Object {$_})
}
$path = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../local-data/stats.json'))
[IO.File]::WriteAllText($path,($catalog | ConvertTo-Json -Depth 20),[Text.UTF8Encoding]::new($false))
Write-Host "Saved artifact stat tables: $path"
