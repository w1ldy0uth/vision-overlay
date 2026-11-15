export function weaponDetails(values, catalog) {
  const reference = catalog.equipmentDetails,
    w = reference?.weapons?.[values.item_id];
  if (!w) return null;
  const level = values.level,
    ascension = values.ascension,
    stage = reference.promotions?.[w.promotionId]?.[ascension],
    curves = reference.curves?.[level];
  const valid =
    Number.isInteger(level) &&
    Number.isInteger(ascension) &&
    stage &&
    level >= 1 &&
    level <= stage.maxLevel &&
    curves;
  const props = w.props.map((p) => {
    const c = curves?.[p.curve];
    return {
      type: p.type,
      value:
        valid && c?.arith === "ARITH_MULTI" && Number.isFinite(c.value)
          ? p.initial * c.value + (stage.props?.[p.type] ?? 0)
          : null,
    };
  });
  return {
    props,
    hasEffect: !!w.effects,
    effect: w.effects?.[values.refinement] ?? null,
    refinement: values.refinement,
  };
}
export function artifactSet(values, catalog) {
  const reference = catalog.equipmentDetails,
    id = reference?.relicSets?.[values.item_id];
  return id && reference.sets?.[id] ? { id, ...reference.sets[id] } : null;
}
export function equipmentTooltip(row, equipped, catalog) {
  if (row.source_collection === "weapons" || row.values.kind === "weapon") {
    const info = weaponDetails(row.values, catalog);
    return {
      name: info?.effect?.name ?? "Эффект оружия",
      description:
        info?.effect?.description ??
        (info?.hasEffect === false
          ? "У оружия нет эффекта пробуждения."
          : "Эффект для текущего ранга пробуждения отсутствует в справочнике."),
    };
  }
  const set = artifactSet(row.values, catalog);
  if (!set)
    return {
      name: "Бонус комплекта",
      description: "Комплект отсутствует в текущем справочнике.",
    };
  const distinct = new Map(equipped.map((r) => [r.entity_key, r]));
  const count = [...distinct.values()].filter(
    (r) => artifactSet(r.values, catalog)?.id === set.id,
  ).length;
  const active = set.bonuses.filter((b) => count >= b.pieces);
  return {
    name: set.name ?? "Бонус комплекта",
    description:
      `Экипировано предметов комплекта: ${count}.\n\n` +
      (active.length
        ? active
            .map(
              (b) =>
                `${b.pieces} предмета: ${b.description ?? "Описание не получено"}`,
            )
            .join("\n\n")
        : `Бонус не активен. Требуется ${set.bonuses[0]?.pieces ?? 2} предмета комплекта.`),
  };
}
export function equipmentStatName(type) {
  const names = {
    BASE_ATTACK: "Базовая атака",
    HP: "HP",
    ATTACK: "Сила атаки",
    DEFENSE: "Защита",
    HEAL_ADD: "Бонус лечения",
    FIRE_ADD_HURT: "Бонус Пиро",
    WATER_ADD_HURT: "Бонус Гидро",
    WIND_ADD_HURT: "Бонус Анемо",
    ICE_ADD_HURT: "Бонус Крио",
    ROCK_ADD_HURT: "Бонус Гео",
    ELEC_ADD_HURT: "Бонус Электро",
    GRASS_ADD_HURT: "Бонус Дендро",
    ATTACK_PERCENT: "Сила атаки",
    HP_PERCENT: "HP",
    DEFENSE_PERCENT: "Защита",
    ELEMENT_MASTERY: "Мастерство стихий",
    CRITICAL: "Шанс крит. попадания",
    CRITICAL_HURT: "Крит. урон",
    CHARGE_EFFICIENCY: "Восстановление энергии",
    PHYSICAL_ADD_HURT: "Бонус физ. урона",
  };
  return names[type?.replace("FIGHT_PROP_", "")] ?? type;
}
export function equipmentStatValue(prop) {
  if (prop.value === null) return "Не определено";
  return /PERCENT|CRITICAL|CHARGE_EFFICIENCY|ADD_HURT|HEAL_ADD/.test(prop.type)
    ? `${(prop.value * 100).toLocaleString("ru-RU", { maximumFractionDigits: 1 })}%`
    : Math.round(prop.value).toLocaleString("ru-RU");
}

export function artifactStats(values, catalog) {
  const mainRef = catalog.mainProps?.find((p) => p.id === values.main_prop_id);
  const rarity =
    catalog.rarities?.[values.item_id] ??
    catalog.items?.[values.item_id]?.rarity;
  const mainValue = catalog.relicLevels
    ?.find((p) => p.rank === rarity && p.level === values.level + 1)
    ?.addProps?.find((p) => p.propType === mainRef?.propType)?.value;
  const totals = new Map();
  let unknownSubstats = 0;
  for (const id of values.append_prop_ids ?? []) {
    const affix = catalog.affixes?.find((a) => a.id === id);
    if (!affix) {
      unknownSubstats++;
      continue;
    }
    totals.set(
      affix.propType,
      (totals.get(affix.propType) ?? 0) + affix.propValue,
    );
  }
  return {
    main: mainRef ? { type: mainRef.propType, value: mainValue ?? null } : null,
    substats: [...totals].map(([type, value]) => ({ type, value })),
    unknownSubstats,
  };
}
