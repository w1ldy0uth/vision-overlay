/** Persistent bonuses come from activated talent IDs and explicit depot actions.
 * Observed extra levels may include conditional +1; never add that map wholesale.
 */
export function talentLevels(profile, catalog) {
  const depot = catalog.skillDepots?.[String(profile.skill_depot_id)];
  const active = new Set((profile.talent_ids ?? []).map(String));
  const bonuses = new Map();
  for (const [talentId, actions] of Object.entries(
    depot?.constellationBonuses ?? {},
  )) {
    if (
      !active.has(talentId) ||
      !(depot.talents ?? []).some((id) => String(id) === talentId)
    )
      continue;
    for (const action of actions) {
      if (action.levels !== 3 || ![3, 5].includes(action.constellation))
        continue;
      const id = String(action.skillId),
        group = catalog.skillMetadata?.[id]?.proudSkillGroupId;
      if (!group || group !== action.proudSkillGroupId) continue;
      const previous = bonuses.get(id);
      if (previous && previous.talentId !== talentId) {
        bonuses.set(id, { unresolved: true });
        continue;
      }
      bonuses.set(id, {
        amount: 3,
        constellation: action.constellation,
        talentId,
      });
    }
  }
  const order = [...(depot?.skills ?? []), depot?.energySkill]
    .filter(Boolean)
    .map(String);
  return Object.entries(profile.skill_levels ?? {})
    .map(([id, base]) => {
      const group = catalog.skillMetadata?.[id]?.proudSkillGroupId;
      const bonus = bonuses.get(id),
        amount = bonus?.amount ?? 0;
      const observed = group
        ? profile.extra_skill_levels?.[String(group)]
        : undefined;
      return {
        id,
        base,
        bonus: amount,
        total: Number(base) + amount,
        constellation: bonus?.constellation,
        conditionalIgnored: observed === amount + 1,
        unresolved:
          !!bonus?.unresolved ||
          (observed !== undefined &&
            observed > amount &&
            observed !== amount + 1) ||
          (!group &&
            Object.values(profile.extra_skill_levels ?? {}).some(
              (v) => v >= 3,
            )),
      };
    })
    .sort(
      (a, b) =>
        (order.includes(a.id) ? order.indexOf(a.id) : 999) -
        (order.includes(b.id) ? order.indexOf(b.id) : 999),
    );
}
