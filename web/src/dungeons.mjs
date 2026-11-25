/**
 * @param {typeof import('./dungeon-schedule.json').dungeons} dungeons
 * @param {number} weekday
 */
export function dungeonsForDay(dungeons, weekday) {
  return dungeons.map(dungeon => ({
    ...dungeon,
    variants: dungeon.variants.filter(variant => variant.days.includes(weekday)),
  })).filter(dungeon => dungeon.variants.length > 0);
}

/**
 * @param {ReturnType<typeof import('./allocation.mjs').allocateGoals>['plans']} plans
 * @param {Set<number>} rewardIds
 */
export function dungeonNeeds(plans, rewardIds) {
  /** @type {Map<number, { goalId: string, values: Record<string, any> | undefined, missing: number | null }[]>} */
  const needs = new Map();
  for (const plan of plans) {
    for (const material of plan.materials) {
      if (!rewardIds.has(material.id) || material.missing === 0) continue;
      const targets = needs.get(material.id) ?? [];
      targets.push({
        goalId: plan.goal.goal_id,
        values: plan.plan.values,
        missing: material.missing,
      });
      needs.set(material.id, targets);
    }
  }
  return needs;
}
