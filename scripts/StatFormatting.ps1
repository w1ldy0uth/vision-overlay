# Dot-source after loading the stat catalog into $stats.
$statNames = @{
 FIGHT_PROP_HP='HP'; FIGHT_PROP_HP_PERCENT='HP'; FIGHT_PROP_ATTACK='Сила атаки'; FIGHT_PROP_ATTACK_PERCENT='Сила атаки'
 FIGHT_PROP_DEFENSE='Защита'; FIGHT_PROP_DEFENSE_PERCENT='Защита'; FIGHT_PROP_CRITICAL='Шанс крит. попадания'
 FIGHT_PROP_CRITICAL_HURT='Крит. урон'; FIGHT_PROP_CHARGE_EFFICIENCY='Восстановление энергии'; FIGHT_PROP_ELEMENT_MASTERY='Мастерство стихий'
 FIGHT_PROP_HEAL_ADD='Бонус лечения'; FIGHT_PROP_PHYSICAL_ADD_HURT='Бонус физ. урона'
 FIGHT_PROP_FIRE_ADD_HURT='Бонус Пиро урона'; FIGHT_PROP_ELEC_ADD_HURT='Бонус Электро урона'; FIGHT_PROP_WATER_ADD_HURT='Бонус Гидро урона'
 FIGHT_PROP_GRASS_ADD_HURT='Бонус Дендро урона'; FIGHT_PROP_WIND_ADD_HURT='Бонус Анемо урона'; FIGHT_PROP_ROCK_ADD_HURT='Бонус Гео урона'; FIGHT_PROP_ICE_ADD_HURT='Бонус Крио урона'
}
function FormatStat($type,$value) {
    $name = if ($statNames.ContainsKey($type)) { $statNames[$type] } else { $type }
    if ($null -eq $value) { return "$name`: ?" }
    $flat = $type -in 'FIGHT_PROP_HP','FIGHT_PROP_ATTACK','FIGHT_PROP_DEFENSE','FIGHT_PROP_ELEMENT_MASTERY'
    $number = if ($flat) { ([double]$value).ToString('F0') } else { (([double]$value)*100).ToString('F1')+'%' }
    return "$name`: $number"
}
$mainProps = @{}; foreach ($prop in $stats.main_props) { $mainProps[[string]$prop.id]=$prop.propType }
$affixes = @{}; foreach ($prop in $stats.affixes) { $affixes[[string]$prop.id]=$prop }
$mainLevels = @{}; foreach ($level in $stats.levels) { $mainLevels["$($level.rank):$($level.level)"]=$level.addProps }
function ArtifactStats($item,$rank) {
    $lines = [Collections.Generic.List[string]]::new()
    $type = $mainProps[[string]$item.main_prop_id]
    $values = @($mainLevels["${rank}:$([int]$item.level+1)"] | Where-Object propType -eq $type)
    if ($type -and $values.Count -eq 1) { $lines.Add((FormatStat $type $values[0].value)) }
    else { $lines.Add("Основной стат ID $($item.main_prop_id): не сопоставлен") }
    $totals = @{}
    foreach ($id in $item.append_prop_ids) {
        $affix = $affixes[[string]$id]
        if ($affix) { $totals[$affix.propType] += [double]$affix.propValue }
        else { $lines.Add("Сабстат ID ${id}: не сопоставлен") }
    }
    foreach ($key in ($totals.Keys | Sort-Object)) { $lines.Add((FormatStat $key $totals[$key])) }
    return $lines -join '; '
}
# Numeric IDs follow EnkaNetwork/API-docs FightProp definitions; unknown IDs stay in raw data.
$fightNames = @{'1'='Базовое HP';'4'='Базовая атака';'7'='Базовая защита';'2000'='Макс. HP';'2001'='Атака';'2002'='Защита';'20'='Шанс крит. попадания';'22'='Крит. урон';'23'='Восстановление энергии';'26'='Бонус лечения';'27'='Бонус получаемого лечения';'28'='Мастерство стихий';'29'='Физ. сопротивление';'30'='Бонус физ. урона'}
$elements = @('Пиро','Электро','Гидро','Дендро','Анемо','Гео','Крио')
for ($i=0;$i -lt 7;$i++) { $fightNames[[string](40+$i)]="Бонус $($elements[$i]) урона"; $fightNames[[string](50+$i)]="$($elements[$i]) сопротивление" }
function CharacterStats($profile) {
    $lines = [Collections.Generic.List[string]]::new()
    foreach ($prop in ($profile.fight_props.PSObject.Properties | Sort-Object {[int]$_.Name})) {
        if ($prop.Name -in '1','4','7' -or !$fightNames.ContainsKey($prop.Name)) { continue }
        $value = [double]$prop.Value
        $text = if ($prop.Name -in '1','4','7','2000','2001','2002','28') { $value.ToString('F0') } else { ($value*100).ToString('F1')+'%' }
        $lines.Add("$(Html $fightNames[$prop.Name]): $(Html $text)")
    }
    if ($profile.friendship) { $lines.Add("Уровень дружбы: $(Html $profile.friendship.level)<br>Опыт дружбы в текущем уровне: $(Html $profile.friendship.exp)") }
    return $lines -join '<br>'
}
