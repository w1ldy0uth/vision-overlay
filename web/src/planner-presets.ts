import { get } from './api';
import { tr } from './i18n.mjs';

export type PlannerPreset = { id: string; name: string };

export function plannerPresetName(preset: PlannerPreset) {
  return preset.id === 'default' && preset.name === 'Основной' ? tr('Основной') : preset.name;
}

export const plannerPresetStorageKey = (profile: string) => `vo.planner.preset.${profile}`;
export const plannerPresetsUrl = (profile: string) => `/api/v1/profiles/${profile}/planner-presets`;
export const plannerGoalsUrl = (profile: string, preset: string) =>
  `/api/v1/profiles/${profile}/goals?preset_id=${encodeURIComponent(preset)}`;

export async function readActivePlannerPreset(profile: string, signal: AbortSignal) {
  const { presets } = await get<{ presets: PlannerPreset[] }>(plannerPresetsUrl(profile), signal);
  const selected = localStorage.getItem(plannerPresetStorageKey(profile)) ?? 'default';
  return presets.find(preset => preset.id === selected) ?? presets.find(preset => preset.id === 'default');
}
