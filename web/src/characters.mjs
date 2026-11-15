export function characterGallery(owned, catalog) {
  const rows = owned.filter(
    (r) =>
      !r.values.trial_avatar_info &&
      ![10000134, 10000135].includes(r.values.avatar_id),
  );
  const ids = new Set(rows.map((r) => Number(r.values.avatar_id)));
  const missing = Object.entries(catalog.avatarMetadata ?? {})
    .filter(([key, meta]) => {
      const id = Number(key);
      return (
        id < 10000900 &&
        ![10000005, 10000007, 10000134, 10000135].includes(id) &&
        meta.useType === "AVATAR_FORMAL" &&
        catalog.avatarNames?.[id] &&
        catalog.avatarIcons?.[id] &&
        !ids.has(id)
      );
    })
    .map(([id]) => ({
      entity_key: `catalog-avatar:${id}`,
      values: { avatar_id: Number(id) },
      unowned: true,
      confirmation: "catalog",
      value_format: "catalog",
      revision: "0",
    }));
  return [...rows, ...missing];
}
export function characterElement(values, catalog) {
  const depot =
    values.skill_depot_id ??
    catalog.avatarMetadata?.[values.avatar_id]?.skillDepotId;
  const skill = catalog.skillDepots?.[depot]?.energySkill;
  return catalog.skillMetadata?.[skill]?.element ?? null;
}
export function characterPriority(a, b, favorites = []) {
  const starred = new Set(favorites);
  return (
    Number(!b.unowned && starred.has(b.values.avatar_id)) -
      Number(!a.unowned && starred.has(a.values.avatar_id)) ||
    Number(!!a.unowned) - Number(!!b.unowned)
  );
}
export function savedFavorites(raw) {
  try {
    const parsed = JSON.parse(raw ?? "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
      return {};
    return Object.fromEntries(
      Object.entries(parsed)
        .filter(([, v]) => Array.isArray(v))
        .map(([k, v]) => [
          k,
          [...new Set(v.filter((n) => Number.isSafeInteger(n) && n > 0))],
        ]),
    );
  } catch {
    return {};
  }
}

export function matchesCharacterElements(row, catalog, selected) {
  return (
    !selected.length || selected.includes(characterElement(row.values, catalog))
  );
}
