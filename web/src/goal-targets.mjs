import { progressionReference } from './progression.mjs';
import { equipmentOwners } from './equipment-owners.mjs';

export function levelTargets(max = 90) {
  const caps = [20, 40, 50, 60, 70, 80];
  return Array.from({length: max}, (_, index) => index + 1).flatMap(level => {
    const ascension = caps.filter(cap => cap < level).length;
    const target = {level, ascension};
    return caps.includes(level) && level < max ? [target, {level, ascension: ascension + 1}] : [target];
  });
}

export function targetIndex(targets, target) {
  const index = targets.findIndex(t => t.level >= target.level && t.ascension >= target.ascension);
  return index === -1 ? targets.length - 1 : index;
}

export function equippedWeapon(data, characterKey) {
  const owners = equipmentOwners(data.collections?.characters);
  return data.collections?.weapons?.find(row => owners.get(row.entity_key)?.entity_key === characterKey);
}

export function maximumWeaponLevel(row, catalog) {
  return progressionReference(row?.values ?? {}, catalog)?.promotions.at(-1)?.maxLevel ?? 90;
}

/** Resolve the weapon on each fresh profile projection. A physical weapon reserves stock once.
 * @returns {{goal: any, target: any, linkedFrom: any}[]}
 */
export function resolveGoalTargets(goals, catalog, data) {
  const entries = goals.map(goal => ({goal, target: goal.values, linkedFrom: null}));
  const weapons = new Map(entries.filter(e => e.target.collection === 'weapons').map(e => [e.target.entity_key, e]));
  for (const goal of goals) {
    const requested = goal.values.equipped_weapon;
    if (goal.values.collection !== 'characters' || !requested) continue;
    const row = equippedWeapon(data, goal.values.entity_key);
    const max = maximumWeaponLevel(row, catalog);
    const targets = levelTargets(max);
    const target = {
      ...targets[targetIndex(targets, requested)],
      priority: goal.values.priority,
      collection: 'weapons', entity_key: row?.entity_key, talents: {},
    };
    const existing = weapons.get(target.entity_key);
    if (existing) {
      existing.target = {...existing.target, level: Math.max(existing.target.level, target.level),
        ascension: Math.max(existing.target.ascension, target.ascension),
        priority: Math.min(existing.target.priority, target.priority)};
      existing.linkedFrom = goal;
    } else {
      const entry = {goal: {...goal, goal_id: goal.goal_id + ':weapon'}, target, linkedFrom: goal};
      entries.push(entry);
      if (row) weapons.set(row.entity_key, entry);
    }
  }
  return entries;
}
