// Equipment belongs to an instance GUID, never to a shared item ID.
export function equipmentOwners(characters = []) {
  const owners = new Map();
  for (const character of characters) {
    if (character.values.trial_avatar_info) continue;
    for (const guid of new Set(character.values.equip_guids ?? [])) {
      owners.set(guid, owners.has(guid) ? null : character);
    }
  }
  return owners;
}
