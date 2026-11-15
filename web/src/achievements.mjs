/** Group only protocol-linked stages; identical titles alone are not evidence of a chain.
 * @param {any[]} rows
 * @param {Record<string, any>} catalog
 */
export function achievementGroups(rows, catalog) {
  const groups = new Map();
  for (const row of rows) {
    const meta = catalog[row.entity_key] ?? {};
    if (meta.excluded) continue;
    const key = meta.chain_id ?? row.entity_key;
    if (!groups.has(key)) groups.set(key, { key, meta, stages: [] });
    groups.get(key).stages.push(row);
  }
  for (const group of groups.values()) {
    group.stages.sort(
      (a, b) =>
        (catalog[a.entity_key]?.stage ?? 1) -
        (catalog[b.entity_key]?.stage ?? 1),
    );
    group.meta = catalog[group.stages[0].entity_key] ?? {};
    const observed = new Map(group.stages.map((row) => [row.entity_key, row]));
    group.levels = Object.entries(catalog)
      .filter(
        ([id, meta]) => !meta.excluded && (meta.chain_id ?? id) === group.key,
      )
      .map(([id, meta]) => ({ id, meta, row: observed.get(id) }))
      .sort((a, b) => (a.meta.stage ?? 1) - (b.meta.stage ?? 1));
    if (!group.levels.length)
      group.levels = group.stages.map((row) => ({
        id: row.entity_key,
        meta: {},
        row,
      }));
  }
  return [...groups.values()].sort(
    (a, b) =>
      (a.meta.category_order ?? 999999) - (b.meta.category_order ?? 999999) ||
      (a.meta.goal_id ?? 999999) - (b.meta.goal_id ?? 999999) ||
      (a.meta.chain_order ?? a.meta.order ?? Number(a.key)) -
        (b.meta.chain_order ?? b.meta.order ?? Number(b.key)) ||
      Number(a.key) - Number(b.key),
  );
}

/** @param {any} group @param {Record<string, any>} catalog */
export function achievementMatches(group, catalog, status, category, search) {
  return (
    (category === "all" ||
      String(group.meta.goal_id ?? "unknown") === category) &&
    group.stages.some(
      (row) =>
        status === "all" ||
        row.values.status_code ===
          { unearned: 1, finished: 2, claimed: 3 }[status],
    ) &&
    group.stages.some((row) => {
      const a = catalog[row.entity_key] ?? {};
      return `${a.name ?? ""} ${a.description ?? ""} ${row.entity_key}`
        .toLocaleLowerCase("ru")
        .includes(search.toLocaleLowerCase("ru"));
    })
  );
}
