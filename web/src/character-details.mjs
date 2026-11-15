import { talentLevels } from "./talents.mjs";
export function characterAbilities(
  values,
  catalog,
  { referenceOnly = false } = {},
) {
  if (referenceOnly)
    values = {
      avatar_id: values.avatar_id,
      skill_depot_id: catalog.avatarMetadata?.[values.avatar_id]?.skillDepotId,
    };
  const depot = catalog.skillDepots?.[values.skill_depot_id],
    reference = catalog.characterDetails;
  const levels = new Map(talentLevels(values, catalog).map((t) => [t.id, t]));
  const ids = [
    ...new Set(
      [...(depot?.skills ?? []), depot?.energySkill, depot?.attackModeSkill]
        .filter(Boolean)
        .map(String),
    ),
  ];
  for (const id of levels.keys()) if (!ids.includes(id)) ids.push(id);
  const active = ids
    .filter(
      (id) =>
        levels.has(id) ||
        reference?.skills?.[id]?.name ||
        catalog.skillNames?.[id],
    )
    .map((id) => ({
      id,
      ...reference?.skills?.[id],
      level: levels.get(id),
    }));
  const rawAscension = values.ascension ?? values.props?.["1002"]?.ival;
  const ascension = rawAscension === undefined ? null : Number(rawAscension);
  const passives = (
    reference?.depots?.[values.skill_depot_id]?.passives ?? []
  ).map((p) => ({
    id: String(p.groupId),
    ...reference.passives?.[p.groupId],
    requiredAscension: p.requiredAscension,
    unlocked: ascension === null ? null : ascension >= p.requiredAscension,
  }));
  const activated = new Set((values.talent_ids ?? []).map(String));
  const constellations = (depot?.talents ?? [])
    .filter(Boolean)
    .map((id, index) => ({
      id: String(id),
      position: index + 1,
      ...reference?.constellations?.[id],
      unlocked: referenceOnly ? null : activated.has(String(id)),
    }));
  return { active, passives, constellations };
}
