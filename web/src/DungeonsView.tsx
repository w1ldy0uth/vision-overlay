import { gameText, tr, uiLocale } from "./i18n.mjs";
import { useEffect, useMemo, useState } from 'react';
import { get, type Profile, type ProfileData } from './api';
import { ItemIcon } from './InventoryViews';
import { allocateGoals } from './allocation.mjs';
import { dungeonNeeds, dungeonsForDay } from './dungeons.mjs';
import schedule from './dungeon-schedule.json';
import { regions } from './model.mjs';
import { plannerGoalsUrl, plannerPresetName, readActivePlannerPreset, type PlannerPreset } from './planner-presets';
import { nextServerWeekReset, serverDay, serverOffsets } from './server-clock.mjs';

type Server = keyof typeof serverOffsets;
type Goal = { goal_id: string; values: Record<string, any> };
type Catalog = {
  items: Record<string, { icon?: string; rarity?: number }>;
  names: Record<string, string>;
  avatarNames: Record<string, string>;
};
const worldRegions: Record<number, string> = {
  1: tr("Мондштадт"), 2: tr("Ли Юэ"), 3: tr("Инадзума"), 4: tr("Сумеру"),
  5: tr("Фонтейн"), 6: tr("Натлан"), 7: tr("Нод-Край"), 8: tr("Снежная"),
};
const weekdays = [tr("Воскресенье"), tr("Понедельник"), tr("Вторник"), tr("Среда"), tr("Четверг"), tr("Пятница"), tr("Суббота")];
const categories = [['talent', tr("Таланты")], ['weapon', tr("Оружие")], ['trounce', tr("Подземелья наказания")]] as const;
const dateTime = new Intl.DateTimeFormat(uiLocale, { dateStyle: 'medium', timeStyle: 'short' });
const count = new Intl.NumberFormat(uiLocale);
const rewardIds = new Set([
  ...schedule.dungeons.flatMap(dungeon => dungeon.variants.flatMap(variant => variant.rewards)),
  ...schedule.trounce.flatMap(dungeon => dungeon.rewards),
]);

export function DungeonsView({ profile, data, catalog, openPlanner }: {
  profile?: Profile;
  data: ProfileData | null;
  catalog: Catalog;
  openPlanner: () => void;
}) {
  const [now, setNow] = useState(Date.now);
  const [manualServer, setManualServer] = useState<Server>('os_euro');
  const [onlyGoals, setOnlyGoals] = useState(false);
  const [category, setCategory] = useState<(typeof categories)[number][0]>('talent');
  const [plan, setPlan] = useState<{ preset: PlannerPreset; goals: Goal[] }>();
  const [planError, setPlanError] = useState(false);
  const hasProfileServer = profile && Object.hasOwn(serverOffsets, profile.server_region);
  const server = hasProfileServer ? profile.server_region as Server : manualServer;
  const { weekday, nextReset } = serverDay(server, now);
  const weekly = category === 'trounce';
  const weekReset = nextServerWeekReset(server, now);

  useEffect(() => {
    const refresh = () => setNow(Date.now());
    const timer = window.setTimeout(refresh, Math.max(0, nextReset - Date.now()));
    window.addEventListener('focus', refresh);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener('focus', refresh);
    };
  }, [nextReset]);

  useEffect(() => {
    const profileId = profile?.profile_id;
    if (!profileId) return;
    const controller = new AbortController();
    const { signal } = controller;
    async function load(profileId: string) {
      const preset = await readActivePlannerPreset(profileId, signal);
      if (!preset) throw Error('Missing planner preset');
      const { goals } = await get<{ goals: Goal[] }>(plannerGoalsUrl(profileId, preset.id), signal);
      if (!signal.aborted) setPlan({ preset, goals });
    }
    void load(profileId).catch(() => { if (!signal.aborted) setPlanError(true); });
    return () => controller.abort();
  }, [profile?.profile_id]);

  const allocation = useMemo(() => plan && data ? allocateGoals(plan.goals, catalog, data) : null,
    [plan, data, catalog]);
  const needs = useMemo(() => dungeonNeeds(allocation?.plans ?? [], rewardIds), [allocation]);
  const entries = weekly
    ? schedule.trounce.map(({ rewards, location, ...boss }) => ({
      ...boss, location, resin: null, variants: [{ label: '', rewards }],
    }))
    : dungeonsForDay(schedule.dungeons, weekday).filter(dungeon => dungeon.kind === category)
      .map(dungeon => ({ ...dungeon, location: undefined }));
  const visible = entries.map(dungeon => ({
    ...dungeon,
    variants: dungeon.variants.filter(variant => !onlyGoals || variant.rewards.some(id => needs.has(id))),
  })).filter(dungeon => dungeon.variants.length > 0);

  return <section className="panel dungeon-panel" aria-label={tr("Подземелья")}>
    <div className="dungeon-toolbar">
      <div>
        <strong>{weekdays[weekday]} · {regions[server]}</strong>
        {!weekly && <div className="muted">{tr("Смена наград ")}<time dateTime={new Date(nextReset).toISOString()}>{dateTime.format(nextReset)}</time></div>}
      </div>
      {!hasProfileServer && <label>{tr("Сервер ")}<select value={server} onChange={event => setManualServer(event.target.value as Server)}>
        {Object.keys(serverOffsets).map(id => <option key={id} value={id}>{regions[id as Server]}</option>)}
      </select></label>}
      {plan && <div className="dungeon-plan-filter">
        <button type="button" className="dungeon-preset-link" onClick={openPlanner}>{tr("Пресет «")}{plannerPresetName(plan.preset)}»</button>
        <label><input type="checkbox" checked={onlyGoals} disabled={!allocation} onChange={event => setOnlyGoals(event.target.checked)} />{tr(" Только для целей")}</label>
      </div>}
    </div>
    {!weekly && <p className="muted dungeon-schedule-note">{tr("Обычная ротация наград. Временные открытия и доступ к уровням сложности смотрите в игре.")}</p>}
    <div className="categories dungeon-categories" role="group" aria-label={tr("Категория подземелий")}>
      {categories.map(([id, label]) => <button key={id} type="button" aria-pressed={category === id}
        className={category === id ? 'selected' : undefined} onClick={() => setCategory(id)}>{label}</button>)}
    </div>
    {weekly && <div className="dungeon-weekly">
      <strong>{tr("30 смолы со скидкой · 60 смолы без скидки")}</strong>
      <span className="muted">{tr("Сброс ")}<time dateTime={new Date(weekReset).toISOString()}>{dateTime.format(weekReset)}</time></span>
    </div>}
    {planError && <p role="status">{tr("Не удалось загрузить цели планировщика.")}</p>}
    {allocation?.partial && <p role="status">{tr("Часть целей пресета пока не удалось рассчитать.")}</p>}
    {visible.length === 0 ? <p role="status">{tr("В этой категории нет недостающих материалов для целей пресета")}{weekly ? '.' : tr(" на сегодня.")}</p> : (
      <div className="dungeon-grid">
        {visible.map(dungeon => <article className="dungeon-card" key={dungeon.id}>
          <div className="dungeon-card-header">
            <div><h2>{gameText(dungeon.name)}</h2><span className="muted">{worldRegions[dungeon.region]}{dungeon.location && ` · ${gameText(dungeon.location)}`}</span></div>
            {dungeon.resin !== null && <span className="dungeon-resin">{dungeon.resin}{tr(" смолы")}</span>}
          </div>
          {dungeon.variants.map(variant => <div className="dungeon-variant" key={variant.label}>
            {variant.label && <h3>{gameText(variant.label)}</h3>}
            <ul className="dungeon-rewards">
              {variant.rewards.map(id => {
                const name = catalog.names[id] ?? tr("Материал {0}", id);
                const item = catalog.items[id];
                const targets = needs.get(id) ?? [];
                return <li key={id} className={targets.length ? 'dungeon-needed' : undefined}>
                  <div className="dungeon-material">
                    <ItemIcon icon={item?.icon} rarity={item?.rarity} />
                    <span>{name}</span>
                  </div>
                  {targets.length > 0 && <ul className="dungeon-targets">
                    {targets.map(target => {
                      const values = target.values ?? {};
                      const name = catalog.avatarNames[values.avatar_id] ?? catalog.names[values.item_id] ?? tr("Цель");
                      return <li key={target.goalId}>{name} <span>{target.missing === null ? tr("· запас неизвестен") : `× ${count.format(target.missing)}`}</span></li>;
                    })}
                  </ul>}
                </li>;
              })}
            </ul>
          </div>)}
        </article>)}
      </div>
    )}
  </section>;
}
