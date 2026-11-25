const groups = [
  ["common", "Общие ресурсы"],
  ["boss", "Материалы обычных боссов"],
  ["mob", "Материалы противников"],
  ["local", "Местные диковинки"],
  ["weapon", "Материалы возвышения оружия"],
  ["books", "Материалы улучшения талантов"],
  ["weekly", "Материалы еженедельных боссов"],
  ["gems", "Камни возвышения персонажей"],
  ["other", "Другие материалы"],
];

/** @template {{id: number}} T @param {T[]} rows @param {any} catalog */
export function groupPlannerResources(rows, catalog) {
  /** @type {Map<string, T[]>} */
  const grouped = new Map(groups.map(([key]) => [key, []]));
  for (const row of rows) {
    const {id} = row;
    const item = catalog.items?.[id];
    const material = catalog.materials?.[id];
    const label = item?.typeLabel ?? "";
    const type = material?.materialType;
    let group = "other";
    if (id === 202 || type === "MATERIAL_EXP_FRUIT" || type === "MATERIAL_WEAPON_EXP_STONE" ||
        id in (catalog.leveling?.avatarMaterials ?? {}) || id in (catalog.leveling?.weaponMaterials ?? {})) {
      group = "common";
    } else if (id >= 113000 && id < 114000) {
      const rarity = item?.rarity ?? material?.rankLevel ?? catalog.rarities?.[id];
      group = Number(rarity) === 5 ? "weekly" : Number(rarity) === 4 ? "boss" : "other";
    } else if ((id >= 112000 && id < 113000) || label === "Улучшение персонажей/оружия") {
      group = "mob";
    } else if (label.startsWith("Диковинка")) {
      group = "local";
    } else if ((id >= 114000 && id < 115000) || label === "Возвышение оружия") {
      group = "weapon";
    } else if (id === 104319 || (label === "Таланты персонажей" && Number(item?.rarity) < 5)) {
      group = "books";
    } else if ((id >= 104100 && id < 104200) || label === "Возвышение персонажей") {
      group = "gems";
    }
    grouped.get(group)?.push(row);
  }
  return groups.map(([key, title]) => ({key, title, rows: grouped.get(key) ?? []})).filter(group => group.rows.length);
}
