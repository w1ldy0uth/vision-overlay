import { tr } from "./i18n.mjs";
import { talentLevels } from "./talents.mjs";
const names = {
  Fire: tr("Пиро"),
  Water: tr("Гидро"),
  Wind: tr("Анемо"),
  Ice: tr("Крио"),
  Rock: tr("Гео"),
  Electric: tr("Электро"),
  Grass: tr("Дендро"),
};
export function isTraveler(v) {
  return [10000005, 10000007].includes(v.avatar_id);
}
export function elementName(depot, catalog) {
  const skill = catalog.skillDepots?.[String(depot)]?.energySkill;
  const element = catalog.skillMetadata?.[String(skill)]?.element;
  return names[element] ?? (skill ? tr("Набор {0}", depot) : tr("Без стихии"));
}
export function travelerSets(v, catalog, data) {
  if (!isTraveler(v)) return [];
  const ids =
    catalog.avatarMetadata?.[String(v.avatar_id)]?.skillDepotIds ?? [];
  return ids
    .filter((id) => catalog.skillDepots?.[String(id)]?.energySkill)
    .map((id) => {
      const active = Number(v.skill_depot_id) === id;
      const row = (data.collections?.traveler_depots ?? []).find(
        (r) =>
          r.values.guid === v.guid &&
          Number(r.values.skill_depot_id) === id &&
          r.values.avatar_id === v.avatar_id,
      );
      const profile = row ? { ...v, ...row.values } : active ? v : null;
      return {
        id,
        active,
        name: elementName(id, catalog),
        known: !!profile,
        saved: !!row && row.confirmation !== "observed",
        constellation: profile?.talent_ids?.length,
        talents: profile ? talentLevels(profile, catalog) : [],
      };
    });
}
