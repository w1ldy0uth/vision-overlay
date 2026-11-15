export function adventureExperience(adventure, reference) {
  const rank = adventure?.rank,
    exp = adventure?.exp;
  if (rank === 60) return "340125/340125";
  const threshold = reference?.adventureExp?.[rank];
  return Number.isSafeInteger(exp) &&
    exp >= 0 &&
    Number.isSafeInteger(threshold) &&
    threshold > 0
    ? `${exp}/${threshold}`
    : null;
}
export function completedAchievements(data, profile, catalog) {
  const completed = (data.collections?.achievements ?? []).filter(
    (r) =>
      [2, 3].includes(r.values.status_code) &&
      !catalog?.[r.entity_key]?.excluded,
  ).length;
  const social = profile?.achievement_count;
  if (Number.isSafeInteger(social) && social >= 0)
    return Math.max(social, completed);
  return completed ||
    data.domains?.some(
      (d) => d.collection === "achievements" && d.scope === "full",
    )
    ? completed
    : null;
}
export function showcaseSlots(showcase, characters) {
  return Array.from({ length: 12 }, (_, i) => {
    const displayed = showcase?.[i];
    if (!displayed) return null;
    const character = characters.find(
      (r) =>
        r.values.avatar_id === displayed.avatar_id &&
        !r.values.trial_avatar_info,
    );
    return { displayed, character };
  });
}
