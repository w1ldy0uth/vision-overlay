import { tr, uiLocale } from "./i18n.mjs";
export const travelerGuideId = 10000005;
const travelerVariantId = 10000007;
const mannequins = [10000117, 10000118];
const alternateTravelers = [10000134, 10000135];
const excludedAvatars = new Set([travelerVariantId, ...mannequins, ...alternateTravelers]);

export type GuideCatalog = {
  avatarMetadata?: Record<string, { useType?: string | null }>;
  avatarNames: Record<string, string>;
  avatarIcons: Record<string, string>;
};

export function guideCharacters(catalog: GuideCatalog) {
  return Object.entries(catalog.avatarMetadata ?? {})
    .map(([key, meta]) => ({ id: Number(key), useType: meta.useType }))
    .filter(({ id, useType }) => useType === "AVATAR_FORMAL" &&
      id > 0 && id < 10000900 && !excludedAvatars.has(id) && catalog.avatarNames[id])
    .map(({ id }) => ({ id, name: catalog.avatarNames[id], icon: catalog.avatarIcons[id] }))
    .sort((a, b) => a.name.localeCompare(b.name, uiLocale));
}

export function guideAvatarId(id: string) {
  return Number(id) === travelerVariantId ? travelerGuideId : Number(id);
}

export const guideSections = [
  ["talents", tr("Таланты")],
  ["weapons", tr("Оружие")],
  ["artifacts", tr("Артефакты")],
  ["stats", tr("Характеристики")],
  ["teams", tr("Отряды")],
] as const;

export type GuideBuild = Partial<Record<(typeof guideSections)[number][0], string[]>> & {
  title?: string;
};

type CharacterGuide = {
  patch?: string;
  overview?: string;
  builds?: GuideBuild[];
};

// Add original guides by avatar ID. Use 10000005 for the shared Traveler guide.
export const characterGuides: Partial<Record<number, CharacterGuide>> = {};
