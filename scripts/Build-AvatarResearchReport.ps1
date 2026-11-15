#requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$InputJsonl,
    [Parameter(Mandatory)][string]$OutputHtml,
    [string]$AvatarCatalog,
    [string]$ItemCatalog
)
$ErrorActionPreference = 'Stop'
if (!$AvatarCatalog) { $AvatarCatalog = Join-Path $PSScriptRoot '../local-data/avatars.ru.json' }
if (!$ItemCatalog) { $ItemCatalog = Join-Path $PSScriptRoot '../local-data/materials.ru.json' }
function Html($value) { [Net.WebUtility]::HtmlEncode([string]$value) }
function NameOf($map,$id) {
    $property = $map.PSObject.Properties[[string]$id]
    if ($property -and $property.Value) { return [string]$property.Value }
    return "ID $id"
}
$avatars = Get-Content -LiteralPath $AvatarCatalog -Raw -Encoding UTF8 | ConvertFrom-Json
$items = Get-Content -LiteralPath $ItemCatalog -Raw -Encoding UTF8 | ConvertFrom-Json
$snapshot = $null
$profiles = $null
$friendUpdates = @{}
$talentUpdates = [Collections.Generic.List[object]]::new()
foreach ($line in [IO.File]::ReadLines((Resolve-Path -LiteralPath $InputJsonl).Path)) {
    if (!$line.Trim()) { continue }
    $event = $line | ConvertFrom-Json
    if ($event.event -eq 'inventory_snapshot') { $snapshot = $event }
    if ($event.event -eq 'item_changed' -and $snapshot -and $event.session -eq $snapshot.session) {
        $change = $event.data.change
        $snapshot.data.items = @($snapshot.data.items | Where-Object { [string]$_.guid -ne [string]$change.guid })
        if ($change.removed -ne $true) { $snapshot.data.items = @($snapshot.data.items) + @($change.item) }
    }
    if ($event.event -in 'avatar_snapshot_candidate','character_snapshot') { $profiles = $event; $friendUpdates = @{}; $talentUpdates.Clear() }
    if ($event.event -eq 'character_added' -and $profiles -and $event.session -eq $profiles.session -and $event.data.profile.trial_info_present -ne $true) {
        $existing = @($profiles.data.profiles | Where-Object { [string]$_.guid -eq [string]$event.data.guid })
        if ($existing.Count -eq 0) { $profiles.data.profiles = @($profiles.data.profiles) + @($event.data.profile) }
    }
    if ($event.event -eq 'constellation_changed' -and $profiles -and $event.session -eq $profiles.session) {
        $matches = @($profiles.data.profiles | Where-Object { [string]$_.guid -eq [string]$event.data.guid -and $_.trial_info_present -ne $true })
        if ($matches.Count -ne 1) { throw 'Missing character for constellation event.' }
        $profile = $matches[0]
        $before = @($profile.talent_ids)
        if ($profile.avatar_id -ne $event.data.avatar_id -or $profile.skill_depot_id -ne $event.data.skill_depot_id -or ($before -join ',') -ne (@($event.data.previous.talent_ids) -join ',')) { throw 'Inconsistent constellation event in character report.' }
        $profile.talent_ids = @($event.data.current.talent_ids)
    }
    if ($event.event -eq 'talent_changed') { $talentUpdates.Add($event) }
    if ($event.event -eq 'friendship_changed') { $friendUpdates[[string]$event.data.guid] = $event }
}
if (!$snapshot -or !$profiles -or $snapshot.session -ne $profiles.session) { throw 'Matching inventory and character snapshots are required.' }
$stats = Get-Content -LiteralPath (Join-Path $PSScriptRoot '../local-data/stats.json') -Raw -Encoding UTF8 | ConvertFrom-Json
. (Join-Path $PSScriptRoot 'StatFormatting.ps1')
foreach ($profile in $profiles.data.profiles) {
    $update = $friendUpdates[[string]$profile.guid]
    if ($update -and $update.session -eq $profiles.session) { $profile.friendship.level = $update.data.current.level; $profile.friendship.exp = $update.data.current.exp }
}
foreach ($update in $talentUpdates) {
    if ($update.session -ne $profiles.session) { continue }
    $matches = @($profiles.data.profiles | Where-Object { [string]$_.guid -eq [string]$update.data.guid -and $_.trial_info_present -ne $true })
    if ($matches.Count -ne 1) { continue }
    $profile = $matches[0]
    $skill = $profile.skill_levels.PSObject.Properties[[string]$update.data.skill_id]
    if (!$skill -or $profile.avatar_id -ne $update.data.avatar_id -or $skill.Value -ne $update.data.previous_level) { throw 'Inconsistent talent event in character report.' }
    $skill.Value = $update.data.level
}
$equipment = @{}
foreach ($item in $snapshot.data.items) { $equipment[[string]$item.guid] = $item }
$depots = @{}
foreach ($depot in $avatars.skill_depots) { $depots[[string]$depot.id] = $depot }
$skills = @{}
foreach ($skill in $avatars.skills) { $skills[[string]$skill.id] = $skill }
$accountProfiles = @($profiles.data.profiles | Where-Object { $_.trial_info_present -ne $true })
$excludedTrials = @($profiles.data.profiles | Where-Object { $_.trial_info_present -eq $true }).Count
if ($profiles.event -eq 'character_snapshot' -and $profiles.data.excluded_trial_count) { $excludedTrials = [int]$profiles.data.excluded_trial_count }
$rows = [Text.StringBuilder]::new()
$missing = 0
$linkedCount = 0
foreach ($profile in ($accountProfiles | Sort-Object avatar_id,avatar_type)) {
    $name = NameOf $avatars.names $profile.avatar_id
    $level = $profile.props.'4001'.ival
    $ascension = $profile.props.'1002'.ival
    if ($null -eq $level) { $level = '?' }
    if ($null -eq $ascension) { $ascension = '?' }
    $depot = $depots[[string]$profile.skill_depot_id]
    $talentIds = @($profile.talent_ids)
    $badTalents = @($talentIds | Where-Object { !$depot -or $_ -notin $depot.talents })
    $constellation = if ($depot -and !$badTalents.Count) { "C$($talentIds.Count)" } else { '?' }
    $skillLines = [Collections.Generic.List[string]]::new()
    foreach ($property in ($profile.skill_levels.PSObject.Properties | Sort-Object Name)) {
        $skill = $skills[$property.Name]
        $skillName = NameOf $avatars.skill_names $property.Name
        $bonus = $null
        if ($skill -and $skill.proudSkillGroupId) {
            $bonusProperty = $profile.extra_skill_levels.PSObject.Properties[[string]$skill.proudSkillGroupId]
            if ($bonusProperty) { $bonus = $bonusProperty.Value }
        }
        $suffix = if ($null -ne $bonus) { " + $bonus" } else { '' }
        $skillLines.Add("$(Html $skillName): $(Html $property.Value)$(Html $suffix) <small>(ID $(Html $property.Name))</small>")
    }
    $weaponLines = [Collections.Generic.List[string]]::new()
    $artifactLines = [Collections.Generic.List[string]]::new()
    $missingHere = 0
    foreach ($guid in $profile.equip_guids) {
        $item = $equipment[[string]$guid]
        if (!$item) { $missing++; $missingHere++; continue }
        $linkedCount++
        $itemName = NameOf $items.item_names $item.item_id
        if ($item.kind -eq 'weapon') { $weaponLines.Add("$(Html $itemName) · ур. $(Html $item.level) · R$(Html $item.refinement) <small>(ID $(Html $item.item_id))</small>") }
        if ($item.kind -eq 'artifact') { $artifactLines.Add("$(Html $itemName) +$(Html $item.level) <small>(ID $(Html $item.item_id))</small><br><small>$(Html (ArtifactStats $item $items.rarities.PSObject.Properties[[string]$item.item_id].Value))</small>") }
    }
    $weapons = if ($weaponLines.Count) { $weaponLines -join '<br>' } else { 'Нет сопоставленного оружия' }
    $artifacts = if ($artifactLines.Count) { $artifactLines -join '<br>' } else { 'Нет сопоставленных артефактов' }
    if ($missingHere) { $artifacts += "<br><strong>Не найдено GUID: $missingHere</strong>" }
    $raw = $profile | ConvertTo-Json -Depth 12
    $search = Html "$name $($profile.avatar_id)"
    [void]$rows.Append("<tr data-search=`"$search`"><td><strong>$(Html $name)</strong><br><small>ID $(Html $profile.avatar_id) · тип $(Html $profile.avatar_type)$(if ($profile.trial_info_present) { ' · метка trial_avatar_info' })</small><details><summary>Сырые поля</summary><pre>$(Html $raw)</pre></details></td><td>$(Html $level) / $(Html $ascension)</td><td>$(Html $constellation)<br><small>$(Html ($talentIds -join ', '))</small></td><td>$($skillLines -join '<br>')<br><small>Набор навыков: $(Html $profile.skill_depot_id)</small></td><td><details><summary>Характеристики</summary>$(CharacterStats $profile)</details></td><td>$weapons<br><details><summary>Артефакты: $($artifactLines.Count)</summary>$artifacts</details></td></tr>")
}
$count = $accountProfiles.Count
$types = ($accountProfiles | Group-Object avatar_type | ForEach-Object { "тип $($_.Name): $($_.Count)" }) -join ' · '
$html = @"
<!doctype html><html lang="ru"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Vision Overlay · Исследование персонажей</title>
<style>:root{color-scheme:dark;font-family:Segoe UI,system-ui;background:#10161e;color:#e4eaf2}body{max-width:1500px;margin:auto;padding:28px}p,small{color:#aab8c9;line-height:1.6}h1{font-size:28px}.notice{border-left:3px solid #d7ad66;padding:14px;background:#201f20}input{font:inherit;background:#1c2633;color:inherit;border:1px solid #3c4b60;border-radius:8px;padding:12px;width:min(600px,90%);margin:12px 0 22px}table{width:100%;border-collapse:collapse}th,td{text-align:left;padding:12px;border-bottom:1px solid #293442;vertical-align:top}th{background:#18212c;position:sticky;top:0}td:nth-child(2),td:nth-child(3){white-space:nowrap}td{font-size:14px}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-width:340px;font-size:11px}summary{cursor:pointer;color:#85d9c2;margin:8px 0}.wrap{overflow:auto}small{font-size:11px}</style>
<h1>Исследование исходного профиля персонажей</h1>
<p class="notice">Сохранённая запись, не живой профиль. Уровни талантов и созвездия учитывают последующие события talent_changed и constellation_changed этой сессии. Уровни, возвышения, созвездия и таланты подтверждены пользователем. Боевые характеристики получены из fight_prop_map на момент снимка; они могут учитывать текущие эффекты. Статы артефактов сопоставлены со статическими таблицами и требуют сверки округления. Уровни талантов показаны как база + наблюдаемый бонус по группе навыка. Источник и условия бонуса пока не определяются: +1 может зависеть от отряда, +3 — от созвездий. Записи с trial_avatar_info исключены из отображения; исходные записи сохранены в диагностическом журнале. Дополнительные варианты Путешественника пока не исключаются. Число записей не приравнено к числу персонажей в архиве.</p>
<p>$count записей после фильтра · исключено пробных: $excludedTrials · $(Html $types) · сопоставлено GUID экипировки: $linkedCount · не найдено: $missing.<br>Источник: $(Html ([IO.Path]::GetFileName([IO.Path]::GetDirectoryName((Resolve-Path -LiteralPath $InputJsonl).Path)))) · каталог $(Html $avatars.revision).</p>
<label for="search">Поиск персонажа</label><br><input id="search" type="search" placeholder="Имя или ID"><div class="wrap"><table><thead><tr><th>Персонаж</th><th>Уровень / возвышение</th><th>Созвездия</th><th>Таланты: база + бонус</th><th>Характеристики из пакета</th><th>Оружие и артефакты</th></tr></thead><tbody>$rows</tbody></table></div>
<script>document.getElementById('search').addEventListener('input',function(){const q=this.value.toLocaleLowerCase('ru').trim();for(const row of document.querySelectorAll('tbody tr'))row.hidden=!row.dataset.search.toLocaleLowerCase('ru').includes(q);});</script></html>
"@
[IO.File]::WriteAllText([IO.Path]::GetFullPath($OutputHtml),$html,[Text.UTF8Encoding]::new($false))
Write-Host "Character report: $count filtered entries, $excludedTrials excluded trials, $linkedCount equipment links, $missing unresolved GUIDs. $OutputHtml"
