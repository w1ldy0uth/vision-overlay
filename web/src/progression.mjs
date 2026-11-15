/** Static reference, indexed by target ascension or target BASE talent level. */
export function progressionReference(values, catalog) {
  const p = catalog.progression;
  if (!p) return null;
  const character = !!values.avatar_id;
  const pathId = character
    ? p.avatarPaths[String(values.avatar_id)]
    : p.weaponPaths[String(values.item_id)];
  const path = (character ? p.avatarPromotions : p.weaponPromotions)[
    String(pathId)
  ];
  if (!path) return null;
  const promotions = Object.entries(path)
    .filter(([stage]) => Number(stage) > 0)
    .map(([stage, step]) => ({ stage: Number(stage), ...step }));
  const depot =
    character && catalog.skillDepots?.[String(values.skill_depot_id)];
  const allowed =
    catalog.avatarMetadata?.[String(values.avatar_id)]?.skillDepotIds;
  const depotMatches =
    !allowed || allowed.includes(Number(values.skill_depot_id));
  // Use the active depot, never every depot associated with the Traveler.
  const skills =
    depot && depotMatches
      ? [
          ...new Set(
            [...(depot.skills ?? []), depot.energySkill].filter(Boolean),
          ),
        ]
      : [];
  const talents = skills.flatMap((id) => {
    const group = catalog.skillMetadata?.[String(id)]?.proudSkillGroupId;
    const levels = p.talentLevels[String(group)];
    if (!levels) return [];
    return [
      {
        skillId: String(id),
        levels: Object.entries(levels).map(([level, step]) => ({
          level: Number(level),
          ...step,
          requiredLevel:
            path[String(step.requiredAscension)]?.requiredLevel ?? null,
        })),
      },
    ];
  });
  return { character, promotions, talents };
}
