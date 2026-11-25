import { localizeGameData, tr, uiLocale } from "./i18n.mjs";
import { useEffect, useMemo, useState } from "react";
import { allocateGoals } from "./allocation.mjs";
import { get, type Profile, type ProfileData } from "./api";
import { plannerGoalsUrl, plannerPresetName, plannerPresetStorageKey, readActivePlannerPreset, type PlannerPreset } from "./planner-presets";
import { groupPlannerResources } from "./planner-resources.mjs";
import { MapMarkerSelector } from "./MapMarkerSelector";
import { WorldMap } from "./WorldMap";
import { floorSources, type FloorGroup, type MapSource, type WorldMapData } from "./world-map";

type Goal = { goal_id: string; values: Record<string, any> };
type PresetGoals = { preset: PlannerPreset; goals: Goal[] };
type MarkerSettings = {
  onlyGoals: boolean;
  material: number;
  world: Record<number, boolean>;
  planner: Record<number, boolean>;
};
const markerStorageKey = "vo.map.marker-settings";
const teleportCategory = 1;
const wideCount = new Intl.NumberFormat(uiLocale);
const locations = [
  [2, tr("Тейват")], [7, tr("Энканомия")], [9, tr("Разлом: Подземные шахты")], [34, tr("Море древности")],
  [36, tr("Древняя Священная гора")], [37, tr("Храм пространства")], [40, tr("Морозная Луна")],
] as const;
const floorAreas: Record<number, string> = {
  1: tr("Мондштадт"), 2: tr("Ли Юэ"), 3: tr("Инадзума"), 4: tr("Сумеру"), 8: tr("Фонтейн"),
  11: tr("Натлан"), 13: tr("Нод-Край"), 16: tr("Снежная"),
};

function readMarkerSettings(): MarkerSettings {
  const defaults = { onlyGoals: false, material: 0, world: {}, planner: {} };
  try {
    const saved = JSON.parse(localStorage.getItem(markerStorageKey) ?? "null");
    if (!saved || typeof saved !== "object") return defaults;
    const choices = (value: unknown) => Object.fromEntries(
      Object.entries(value && typeof value === "object" ? value : {})
        .filter(([id, visible]) => /^\d+$/.test(id) && typeof visible === "boolean"),
    );
    return {
      onlyGoals: saved.onlyGoals === true,
      material: Number.isSafeInteger(saved.material) && saved.material > 0 ? saved.material : 0,
      world: choices(saved.world),
      planner: choices(saved.planner),
    };
  } catch {
    return defaults;
  }
}

export function WorldMapView({ profile, data, catalog, openPlanner }: {
  profile?: Profile;
  data: ProfileData | null;
  catalog: { names: Record<string, string>; progression?: unknown };
  openPlanner: () => void;
}) {
  const [mapId, setMapId] = useState(2);
  const [loadedMap, setMap] = useState<WorldMapData>();
  const [mapError, setMapError] = useState(false);
  const [floorGroup, setFloorGroup] = useState(-1);
  const [selectedFloor, setSelectedFloor] = useState(0);
  const [attempt, setAttempt] = useState(0);
  const [plan, setPlan] = useState<PresetGoals>();
  const [planError, setPlanError] = useState(false);
  const [markers, setMarkers] = useState(readMarkerSettings);
  const profileId = profile?.profile_id;
  const filtering = Boolean(profileId && markers.onlyGoals);
  const markerMode = filtering ? "planner" : "world";
  const map = loadedMap?.meta.mapId === mapId ? loadedMap : undefined;
  const group = map?.floorGroups.find(entry => entry.id === floorGroup);
  const floor = group?.levels.find(level => level.id === selectedFloor);
  const floorId = floor?.id ?? (floorGroup === 0 ? 0 : null);
  const sourcesOnFloor = useMemo(() => floorSources(map?.sources ?? [], floorId), [map, floorId]);
  const hiddenSources = useMemo(() => new Set((map?.sources ?? [])
    .filter(source => !(markers[markerMode][source.id] ?? (filtering || source.category === teleportCategory)))
    .map(source => source.id)), [map, markers, markerMode, filtering]);
  const groupsByArea = useMemo(() => {
    const sorted = [...(map?.floorGroups ?? [])].sort((a, b) => a.name.localeCompare(b.name, uiLocale));
    const areas = new Map<number, FloorGroup[]>();
    for (const group of sorted) {
      const area = mapId === 2 ? group.area : 0;
      const entries = areas.get(area);
      if (entries) entries.push(group);
      else areas.set(area, [group]);
    }
    return [...areas].sort((a, b) => a[0] - b[0]);
  }, [map, mapId]);

  useEffect(() => {
    const controller = new AbortController();
    const filename = mapId === 2 ? "world-map.json" : `world-map-${mapId}.json`;
    get<WorldMapData>(`/${filename}`, controller.signal)
      .then(payload => setMap(localizeGameData(payload)))
      .catch(() => { if (!controller.signal.aborted) setMapError(true); });
    return () => controller.abort();
  }, [mapId, attempt]);

  useEffect(() => {
    if (!profileId) return;
    const storageKey = plannerPresetStorageKey(profileId);
    let request: AbortController;
    async function load(profileId: string) {
      request?.abort();
      request = new AbortController();
      const { signal } = request;
      try {
        const preset = await readActivePlannerPreset(profileId, signal);
        if (!preset) throw Error("Missing planner preset");
        const { goals } = await get<{ goals: Goal[] }>(plannerGoalsUrl(profileId, preset.id), signal);
        if (!signal.aborted) {
          setPlan({ preset, goals });
          setPlanError(false);
        }
      } catch {
        if (!signal.aborted) {
          setPlan(undefined);
          setPlanError(true);
        }
      }
    }
    const refresh = () => { void load(profileId); };
    const storage = (event: StorageEvent) => { if (event.key === storageKey) refresh(); };
    refresh();
    window.addEventListener("focus", refresh);
    window.addEventListener("storage", storage);
    return () => {
      request.abort();
      window.removeEventListener("focus", refresh);
      window.removeEventListener("storage", storage);
    };
  }, [profileId, attempt]);

  const needs = useMemo(() => {
    if (!plan || !data || !catalog.progression) return null;
    const allocation = allocateGoals(plan.goals, catalog, data);
    return new Map(allocation.materials.filter(row => row.missing !== 0).map(row => [row.id, row.missing]));
  }, [plan, data, catalog]);

  const unmapped = useMemo(() => {
    if (!filtering || !needs || !map) return [];
    const covered = new Set(map.sources.flatMap(source => source.items));
    const rows = [...needs.keys()].filter(id => !covered.has(id)).map(id => ({ id }));
    return groupPlannerResources(rows, catalog).filter(group => ["local", "mob", "boss"].includes(group.key))
      .flatMap(group => group.rows);
  }, [filtering, needs, map, catalog]);
  const matchingSources = useMemo(() => sourcesOnFloor.filter(source =>
    !hiddenSources.has(source.id) &&
    (!filtering || source.items.some(id => needs?.has(id)))), [sourcesOnFloor, hiddenSources, filtering, needs]);
  const options = useMemo(() => [...new Set(matchingSources.flatMap(source => source.items))]
    .filter(id => !filtering || needs?.has(id))
    .sort((a, b) => materialName(a).localeCompare(materialName(b), uiLocale)), [matchingSources, filtering, needs, catalog.names]);
  const materialFilter = options.includes(markers.material) ? markers.material : 0;
  const sources = useMemo(() => materialFilter ? matchingSources.filter(source => source.items.includes(materialFilter)) :
    matchingSources, [matchingSources, materialFilter]);

  function materialName(id: number) { return catalog.names[id] ?? String(id); }
  function saveMarkers(next: MarkerSettings) {
    setMarkers(next);
    try { localStorage.setItem(markerStorageKey, JSON.stringify(next)); } catch {}
  }
  function setMaterial(material: number) {
    saveMarkers({ ...markers, material });
  }
  function showSources(sources: MapSource[], visible: boolean) {
    const next = { ...markers[markerMode] };
    for (const source of sources) {
      const initiallyVisible = filtering || source.category === teleportCategory;
      if (visible === initiallyVisible) delete next[source.id];
      else next[source.id] = visible;
    }
    saveMarkers({ ...markers, [markerMode]: next });
  }
  function retry() {
    setMapError(false);
    setPlanError(false);
    setAttempt(n => n + 1);
  }

  return <section className="panel world-map-panel" aria-label={tr("Интерактивная карта Тейвата")}>
    <div className="world-map-toolbar">
      <select aria-label={tr("Локация")} value={mapId} onChange={event => {
        setMapError(false);
        setMapId(Number(event.target.value));
        setFloorGroup(-1);
        setSelectedFloor(0);
        setMaterial(0);
      }}>
        {locations.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
      </select>
      {!!map?.floorGroups.length && <select aria-label={tr("Область и ярусы")} value={floorGroup} onChange={event => {
        const id = Number(event.target.value);
        setFloorGroup(id);
        setSelectedFloor(map.floorGroups.find(group => group.id === id)?.defaultId ?? 0);
        setMaterial(0);
      }}>
        <option value={-1}>{tr("Все ярусы")}</option>
        <option value={0}>{tr("Поверхность")}</option>
        {groupsByArea.map(([area, groups]) => <optgroup key={area}
          label={mapId === 2 ? floorAreas[area] ?? tr("Другие области") : tr("Многоуровневые области")}>
          {groups.map(group => <option key={group.id} value={group.id}>{group.name}</option>)}
        </optgroup>)}
      </select>}
      {group && <select aria-label={tr("Ярус")} value={selectedFloor} onChange={event => {
        setSelectedFloor(Number(event.target.value)); setMaterial(0);
      }}>
        {group.levels.map(level => <option key={level.id} value={level.id}>
          {level.short && level.short !== level.name ? `${level.short} · ` : ""}{level.name}
        </option>)}
      </select>}
      {map && <MapMarkerSelector key={mapId} categories={map.categories} sources={map.sources}
        hidden={hiddenSources} show={showSources} />}
      <label><input type="checkbox" checked={filtering} disabled={!profileId}
        onChange={event => saveMarkers({ ...markers, onlyGoals: event.target.checked, material: 0 })} />{tr(" По активному пресету")}</label>
      {plan && <button type="button" className="world-map-preset" onClick={openPlanner}>{plannerPresetName(plan.preset)}</button>}
      <select aria-label={tr("Материал")} value={materialFilter} onChange={event => setMaterial(Number(event.target.value))}>
        <option value={0}>{tr("Все материалы")}</option>
        {options.map(id => <option key={id} value={id}>
          {materialName(id)}{filtering ? (needs?.get(id) == null ? tr(" · запас неизвестен") : tr(" · нужно ещё {0}", wideCount.format(needs.get(id) ?? 0))) : ""}
        </option>)}
      </select>
    </div>
    {filtering && (planError ? <p role="alert">{tr("Не удалось загрузить пресет. ")}<button onClick={retry}>{tr("Повторить")}</button></p> :
      !needs ? <p role="status">{tr("Загружаем потребности пресета…")}</p> :
      map && !matchingSources.length ? <p role="status">{tr("Для выбранных типов нет меток по текущему дефициту.")}</p> : null)}
    {map ? <WorldMap key={mapId} profileId={profileId} meta={map.meta} sources={sources} material={materialFilter} floor={floor}
      needs={filtering ? needs : null} names={catalog.names} /> :
      <div className="empty">{mapError ? <>{tr("Данные карты недоступны. ")}<button onClick={retry}>{tr("Повторить")}</button></> : tr("Загружаем карту…")}</div>}
    {map && <footer className="world-map-attribution muted">
      <span>{map.meta.mapName} · {map.meta.gameVersion} · {wideCount.format(sources.reduce((n, source) => n + source.points.length, 0))}{tr(" меток")}</span>
      <a href="https://act.hoyolab.com/ys/app/interactive-map/index.html" target="_blank" rel="noreferrer">{tr("Карта HoYoLAB")}</a>
    </footer>}
    {unmapped.length > 0 && <details className="world-map-unmapped muted">
      <summary>{tr("Материалы пресета без меток на этой карте (")}{unmapped.length})</summary>
      <ul>{unmapped.map(({ id }) => <li key={id}>{materialName(id)}</li>)}</ul>
    </details>}
  </section>;
}
