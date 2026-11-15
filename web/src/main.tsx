import {DiagnosticExport} from "./DiagnosticExport";
import {artifactStats} from "./equipment-details.mjs";
import { WeaponDetails, ArtifactSetDetails } from "./EquipmentDetails";
import { CharacterDetails } from "./CharacterDetails";
import { CharacterGrid, elements } from "./CharacterGrid";
import {characterGallery,characterPriority,savedFavorites,matchesCharacterElements} from "./characters.mjs";
import { equipmentOwners } from "./equipment-owners.mjs";
import {gameSession,selectedProfile} from "./profile-selection.mjs";
import {useInventoryWindow} from "./useInventoryWindow";
import { Planner } from "./PlanningView";
import { Wishes } from "./Wishes";
import { talentLevels } from "./talents.mjs";
import { ProgressionReference } from "./ProgressionReference";
import { NextUpgrades } from "./NextUpgrades";
import { TravelerSets } from "./TravelerSets";
import { isTraveler, elementName } from "./traveler.mjs";
import { Maintenance } from "./Maintenance";
import { Backups } from "./Backups";
import { CatalogPanel } from "./CatalogPanel";
import { ItemIcon, InventoryGrid, CatalogIconVersion } from "./InventoryViews";
import { achievementGroups, achievementMatches } from "./achievements.mjs";
import { PlayerProfile, Teams } from "./AccountViews";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { get, pages, type Row, type ProfileData, type State } from "./api";
import {
  aggregate,
  characterLevel,
  characterConstellation,
  collections,
  compareEntities,
  freshness,
  newer,
  regions,
  relevant,
  describeOperation,
} from "./model.mjs";
import {
  inventoryCategories,
  developerInventoryCategories,
  groupInventory,
  categoryDomain,
  compareGameOrder,
  itemSubtype,
  subtypeOptions,
} from "./inventory.mjs";
import "./style.css";

type Catalog = {
  equipmentDetails?: any;
  characterDetails?: any;
  progression?: any;
  leveling?: any;
  skillMetadata?: Record<string, any>;
  skillDepots?: Record<string, any>;
  avatarMetadata?: Record<string, any>;
  items: Record<string, any>;
  avatarIcons: Record<string, string>;
  materials: Record<
    string,
    {
      itemType: string;
      materialType: string | null;
      systemType?: string | null;
    }
  >;
  names: Record<string, string>;
  rarities: Record<string, number>;
  avatarNames: Record<string, string>;
  skillNames: Record<string, string>;
  achievements: Record<string, any>;
  mainProps: any[];
  relicLevels: any[];
  affixes: any[];
};
const emptyCatalog: Catalog = {
  items: {},
  avatarIcons: {},
  materials: {},
  names: {},
  rarities: {},
  avatarNames: {},
  skillNames: {},
  achievements: {},
  mainProps: [],
  relicLevels: [],
  affixes: [],
};
const labels = collections as Record<string, string>,
  regionNames = regions as Record<string, string>,
  freshnessNames = freshness as Record<string, string>;
const fmt = (v: unknown) =>
  v === null || v === undefined ? "—" : Number(v).toLocaleString("ru-RU");
const date = (v: unknown) =>
  v ? new Date(Number(v)).toLocaleString("ru-RU") : "Время неизвестно";
const propNames: Record<string, string> = {
  HP: "HP",
  HP_PERCENT: "HP",
  ATTACK: "Сила атаки",
  ATTACK_PERCENT: "Сила атаки",
  DEFENSE: "Защита",
  DEFENSE_PERCENT: "Защита",
  CRITICAL: "Шанс крит. попадания",
  CRITICAL_HURT: "Крит. урон",
  CHARGE_EFFICIENCY: "Восст. энергии",
  ELEMENT_MASTERY: "Мастерство стихий",
  HEAL_ADD: "Бонус лечения",
  FIRE_ADD_HURT: "Бонус Пиро",
  WATER_ADD_HURT: "Бонус Гидро",
  ICE_ADD_HURT: "Бонус Крио",
  ELEC_ADD_HURT: "Бонус Электро",
  WIND_ADD_HURT: "Бонус Анемо",
  ROCK_ADD_HURT: "Бонус Гео",
  GRASS_ADD_HURT: "Бонус Дендро",
  PHYSICAL_ADD_HURT: "Бонус физ. урона",
};
function statName(type: string) {
  return propNames[type.replace("FIGHT_PROP_", "")] ?? type;
}
function statValue(type: string, v: number) {
  return /PERCENT|CRITICAL|CHARGE_EFFICIENCY|ADD_HURT|HEAL_ADD/.test(type)
    ? `${(v * 100).toLocaleString("ru-RU", { maximumFractionDigits: 1 })}%`
    : fmt(Math.round(v));
}
function useApp() {
  const [state, setState] = useState<State | null>(null),
    [selection,setSelection]=useState<{profileId:string;manualSession:string|null}>(()=>{
      try {const saved=JSON.parse(sessionStorage.getItem("vo.profile.browse")??"null");if(saved&&typeof saved.profileId==="string"&&(saved.manualSession===null||typeof saved.manualSession==="string"))return saved;}catch{}
      return {profileId:localStorage.getItem("vo.profile")??"",manualSession:null};
    });
  useEffect(()=>{sessionStorage.setItem("vo.profile.browse",JSON.stringify(selection));},[selection]);
  const selected=selectedProfile(selection,state);
  const setSelected=(profileId:string)=>setSelection({profileId,manualSession:gameSession(state)});
  const followGame=()=>setSelection({profileId:state?.active_game_profile_id??selected,manualSession:null});
  const [connected, setConnected] = useState(false),
    [error, setError] = useState(""),
    [tick, setTick] = useState(0);
  const refs = useRef({ selected });
  refs.current = { selected };
  useEffect(() => {
    localStorage.setItem("vo.profile", selected);
  }, [selected]);
  useEffect(()=>{
    if(selected&&selected!==selection.profileId)setSelection({profileId:selected,manualSession:null});
  },[selected,selection.profileId]);
  useEffect(() => {
    localStorage.removeItem("vo.follow");
  }, []);
  useEffect(() => {
    let disposed = false,
      stream: EventSource | undefined,
      timer: ReturnType<typeof setTimeout> | undefined,
      refresh: ReturnType<typeof setTimeout> | undefined,
      last = "0";
    function publish(s: State) {
      if (disposed) return;
      setState((old) => (!old || !newer(old.revision, s.revision) ? s : old));
    }
    async function boot() {
      try {
        const s = await get<State>("/api/v1/state");
        if (disposed) return;
        publish(s);
        last = s.revision;
        setTick((n) => n + 1);
        setError("");
        stream = new EventSource("/api/v1/events?after=" + last);
        stream.onopen = () => {
          setConnected(true);
          setError("");
        };
        stream.addEventListener("change", (e) => {
          try {
            const change = JSON.parse((e as MessageEvent).data);
            if (!newer(change.revision, last)) return;
            last = change.revision;
            if (relevant(change, refs.current.selected)) setTick((n) => n + 1);
            if (!refresh)
              refresh = setTimeout(() => {
                refresh = undefined;
                get<State>("/api/v1/state")
                  .then(publish)
                  .catch(() => {});
              }, 200);
          } catch {
            reconnect();
          }
        });
        stream.addEventListener("resync_required", reconnect);
        stream.onerror = reconnect;
      } catch (e) {
        if (disposed) return;
        setConnected(false);
        setError(String(e));
        timer = setTimeout(boot, 2000);
      }
    }
    function reconnect() {
      stream?.close();
      setConnected(false);
      if (!disposed) {
        clearTimeout(timer);
        timer = setTimeout(boot, 2000);
      }
    }
    void boot();
    return () => {
      disposed = true;
      stream?.close();
      clearTimeout(timer);
      clearTimeout(refresh);
    };
  }, []);
  const [data, setData] = useState<ProfileData | null>(null),
    [dataError, setDataError] = useState(""),
    [loading, setLoading] = useState(false);
  useEffect(() => {
    if (!selected) return;
    const controller = new AbortController();
    setLoading(true);
    setDataError("");
    const timer = setTimeout(() => {
      get<ProfileData>("/api/v1/profiles/" + selected, controller.signal)
        .then((v) => {
          setData(v);
          setLoading(false);
        })
        .catch((e) => {
          if (!controller.signal.aborted) {
            setDataError(String(e));
            setLoading(false);
          }
        });
    }, 100);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [selected, tick]);
  return {
    state,
    selected,
    setSelected,
    followGame,
    connected,
    error,
    data: data?.profile_id === selected ? data : null,
    dataError,
    loading,
    reload: () => setTick((n) => n + 1),
    tick,
  };
}
function App() {
  const [settingsOpen,setSettingsOpen]=useState(false);
  const [copied, setCopyState] = useState(false),
    [copyError, setCopyErrorState] = useState(false);
  const app = useApp(),
    [catalog, setCatalog] = useState<Catalog>(emptyCatalog),
    [catalogError, setCatalogError] = useState(false),
    [catalogTick, setCatalogTick] = useState(0),
    [tab, setTab] = useState("inventory"),
    [category, setCategory] = useState(() => {
      const saved = localStorage.getItem("vo.inventory.category");
      return inventoryCategories.some(([id]) => id === saved)
        ? saved!
        : "development";
    }),
    [inventoryDeveloper, setInventoryDeveloper] = useState(false),
    [search, setSearch] = useState(
      localStorage.getItem("vo.inventory.search") ?? "",
    ),
    [sort, setSort] = useState(
      localStorage.getItem("vo.inventory.sort") ?? "game",
    ),
    [inventoryView, setInventoryView] = useState(
      localStorage.getItem("vo.inventory.view") === "list" ? "list" : "grid",
    ),
    [inventoryFilters, setInventoryFilters] = useState<
      Record<string, { subtype?: string; rarity?: string }>
    >(() => {
      try {
        return (
          JSON.parse(localStorage.getItem("vo.inventory.filters") ?? "{}") ?? {}
        );
      } catch {
        return {};
      }
    }),
    [detail, setDetail] = useState<Row | null>(null);
  const [detailParent,setDetailParent]=useState<Row|null>(null);
  const [selectedElements,setSelectedElements]=useState<string[]>([]);
  const [favoritesByProfile,setFavoritesByProfile] = useState<Record<string,number[]>>(()=>savedFavorites(localStorage.getItem("vo.characters.favorites")));
  const favorites = favoritesByProfile[app.selected] ?? [];
  const toggleFavorite = (id:number) => setFavoritesByProfile(all=>{
    const existing=all[app.selected]??[];
    return {...all,[app.selected]:existing.includes(id)?existing.filter(n=>n!==id):[...existing,id]};
  });
  useEffect(()=>{localStorage.setItem("vo.characters.favorites",JSON.stringify(favoritesByProfile));},[favoritesByProfile]);
  const catalogVersion = useRef("");
  useEffect(() => {
    let disposed = false,
      running = false;
    const controller = new AbortController();
    async function load(force = false) {
      if (running) return;
      running = true;
      try {
        if (!force) {
          const status = await get("/api/v1/catalog", controller.signal);
          if (status.current?.id === catalogVersion.current) return;
        }
        const response = await fetch("/catalog.json", {
          cache: "no-store",
          signal: controller.signal,
        });
        if (!response.ok) throw new Error("Catalog unavailable");
        const value = (await response.json()) as Catalog;
        if (!disposed) {
          catalogVersion.current =
            response.headers.get("x-catalog-version") ?? "bundled";
          setCatalog(value);
          setCatalogError(false);
        }
      } catch {
        if (!disposed && !catalogVersion.current) setCatalogError(true);
      } finally {
        running = false;
      }
    }
    void load(true);
    const timer = setInterval(() => void load(), 5000);
    const focus = () => void load();
    window.addEventListener("focus", focus);
    return () => {
      disposed = true;
      controller.abort();
      clearInterval(timer);
      window.removeEventListener("focus", focus);
    };
  }, [catalogTick]);
  useEffect(() => {
    localStorage.setItem("vo.inventory.sort", sort);
    localStorage.setItem("vo.inventory.view", inventoryView);
    localStorage.setItem(
      "vo.inventory.filters",
      JSON.stringify(inventoryFilters),
    );
  }, [sort, inventoryView, inventoryFilters]);
  useEffect(() => {
    if (tab === "inventory") {
      localStorage.setItem("vo.inventory.search", search);
      localStorage.setItem("vo.inventory.category", category);
    }
  }, [search, category, tab]);
  const subtype =
    tab === "inventory" ? (inventoryFilters[category]?.subtype ?? "") : "";
  const rarityFilter =
    tab === "inventory" ? (inventoryFilters[category]?.rarity ?? "") : "";
  const setInventoryFilter = (key: "subtype" | "rarity", value: string) =>
    setInventoryFilters((all) => ({
      ...all,
      [category]: { ...all[category], [key]: value },
    }));
  useEffect(() => {
    setDetail(null);
    setDetailParent(null);
  }, [
    app.selected,
    tab,
    category,
    search,
    sort,
    subtype,
    rarityFilter,
    inventoryView,
  ]);
  const name = (v: Record<string, any>) =>
    v.avatar_id
      ? (catalog.avatarNames[String(v.avatar_id)] ?? `Персонаж ${v.avatar_id}`)
      : (catalog.names[String(v.item_id)] ??
        `Предмет ${v.item_id ?? "без ID"}`);
  const data = app.data,
    profile = app.state?.profiles.find((p) => p.profile_id === app.selected),
    current = app.state?.active_game_profile_id === app.selected;
  // Verified separate crossbow-mode avatar; keep raw profile facts for diagnostics.
  const roster = useMemo(
    () =>
      (data?.collections.characters ?? []).filter(
        (r) => r.values.avatar_id !== 10000134,
      ),
    [data?.collections.characters],
  );
  const inventory = useMemo(
    () => groupInventory(data?.collections ?? {}, catalog.materials ?? {}),
    [data, catalog],
  );
  const gallery = useMemo(()=>characterGallery(roster,catalog),[roster,catalog]);
  const rawRows: Row[] =
    tab === "characters" ? gallery : (inventory[category] ?? []);
  const rows = useMemo(() => {
    let list: Row[] = rawRows;
    if(tab === "characters") list = list.filter(r=>matchesCharacterElements(r,catalog,selectedElements));
    if (tab === "inventory")
      list = list.filter(
        (r) =>
          (!subtype || itemSubtype(r, catalog.items ?? {}) === subtype) &&
          (!rarityFilter ||
            String(
              catalog.rarities[String(r.values.item_id)] ??
                catalog.items?.[String(r.values.item_id)]?.rarity ??
                0,
            ) === rarityFilter),
      );
    list = list.filter((r) =>
      [
        name(r.values),
        r.entity_key,
        (r.guids ?? []).join(" "),
        String(r.values.item_id ?? r.values.avatar_id),
      ].some((v) =>
        v.toLocaleLowerCase("ru").includes(search.toLocaleLowerCase("ru")),
      ),
    );
    return [...list].sort((a, b) =>
      (tab === "characters" ? characterPriority(a,b,favorites) : 0) || (sort === "game" && tab === "inventory"
        ? compareGameOrder(a, b, catalog.items ?? {})
        : sort === "count" && tab === "inventory"
          ? Number(b.values.count ?? 0) - Number(a.values.count ?? 0)
          : sort === "level"
            ? characterLevel(b.values) - characterLevel(a.values) ||
              Number(b.values.level ?? 0) - Number(a.values.level ?? 0)
            : name(a.values).localeCompare(name(b.values), "ru")),
    );
  }, [rawRows, category, tab, search, sort, catalog, subtype, rarityFilter, favoritesByProfile, app.selected, selectedElements]);
  const windowKey=JSON.stringify([app.selected,tab,category,search,sort,subtype,rarityFilter,inventoryView,selectedElements]);
  const virtual=useInventoryWindow(rows,windowKey);
  const shown=virtual.shown,
    domain =
      tab === "characters"
        ? data?.domains.find((d) => d.collection === "characters")
        : categoryDomain(category, rawRows, data?.domains ?? []);
  const wallet = Object.fromEntries(
    (data?.collections.wallet ?? []).map((r) => [r.entity_key, r.values.count]),
  );
  return (
    <CatalogIconVersion.Provider value={catalogVersion.current}>
      <div className="layout">
        <aside className="sidebar">
          <a className="brand" href="/" aria-label="Vision Overlay, главная">
            <span className="brand-mark">◇</span>
            <span>
              VISION<span className="brand-sub">OVERLAY</span>
            </span>
          </a>
          <nav>
            {[
              ["profile", "◈", "Профиль"],
              ["teams", "♧", "Отряды"],
              ["inventory", "▦", "Инвентарь"],
              ["characters", "♙", "Персонажи"],
              ["achievements", "✧", "Достижения"],
              ["wishes", "✦", "Молитвы"],
              ["planner", "◎", "Планировщик"],
            ].map(([id, icon, title]) => (
              <button
                key={id}
                className={tab === id ? "nav active" : "nav"}
                onClick={() => {
                  setTab(id);
                  setSearch("");
                }}
              >
                <span>{icon}</span>
                {title}
              </button>
            ))}
            <details className="settings-nav" open={settingsOpen} onToggle={e=>setSettingsOpen(e.currentTarget.open)}>
              <summary className={`nav settings-toggle${["history","captures","backups","maintenance","catalog"].includes(tab)?" active":""}`}><span>⚙</span>Настройки</summary>
              <div className="settings-subnav">{[["history","↔","История изменений"],["captures","◷","Захваты и снимки"],["backups","⇩","Резервные копии"],["maintenance","◴","Хранение данных"],["catalog","⌘","Справочник"]].map(([id,icon,title])=><button key={id} className={tab===id?"nav active":"nav"} onClick={()=>{setTab(id);setSearch("");}}><span>{icon}</span>{title}</button>)}</div>
            </details>
          </nav>
          <div className="sidebar-bottom">
            <span className={"dot " + (app.connected ? "green" : "amber")} />
            {app.connected ? "Связь с приложением" : "Восстанавливаем связь"}
            <small>Локальное хранилище · SQLite</small>
          </div>
        </aside>
        <main>
          <header className="topbar">
            <div>
              <span className="eyebrow">GENSHIN IMPACT</span>
              <div className="profile-picker">
                <select
                  aria-label="Профиль"
                  value={app.selected}
                  title="Просмотр сохранённых профилей. При новом входе профиль переключится автоматически."
                  onChange={e=>app.setSelected(e.target.value)}
                >
                  {app.state?.profiles.map((p) => (
                    <option key={p.profile_id} value={p.profile_id}>
                      {regionNames[p.server_region] ?? p.server_region} ·{" "}
                      {p.uid}
                    </option>
                  ))}
                </select>
                <span className={"pill " + (current ? "live" : "")}>
                  {current ? "В игре" : "Сохранённый профиль"}
                </span>
                {!current&&app.state?.active_game_profile_id&&<button className="follow-game" onClick={app.followGame}>К аккаунту в игре</button>}
              </div>
            </div>
            <div className="header-stats" aria-label="Ресурсы выбранного профиля">
              <Stat label="Мора" value={fmt(wallet["202"])} symbol="◈" />
              <Stat label="Камни Истока" value={fmt(wallet["201"])} symbol="✦" />
              <Stat label="Ранг приключений" value={fmt(data?.collections.adventure?.[0]?.values.rank)} symbol="⌁" />
            </div>
            <button
              className="icon-button"
              onClick={app.reload}
              title="Обновить данные"
              aria-label="Обновить данные"
            >
              ↻
            </button>
          </header>
          <div className="content">
            <div className="heading">
              <div>
                <h1>
                  {
                    {
                      profile: "Профиль игрока",
                      teams: "Сохранённые отряды",
                      inventory: "Инвентарь",
                      characters: "Персонажи",
                      achievements: "Достижения",
                      wishes: "Молитвы",
                      planner: "Планировщик",
                      history: "История изменений",
                      captures: "Захваты и снимки",
                      backups: "Резервные копии",
                      maintenance: "Хранение данных",
                      catalog: "Справочник",
                    }[tab]
                  }
                </h1>
                <p>
                  {tab === "inventory"
                    ? "Собранные ресурсы и экипировка вашего аккаунта."
                    : tab === "characters"
                      ? "Характеристики, таланты и экипировка персонажей."
                      : tab === "history"
                        ? "Изменения, сохранённые приложением."
                        : "Сохранённый прогресс и данные захватов."}
                </p>
              </div>
              <div className="revision">
                Ревизия <b>{data?.revision ?? app.state?.revision ?? "—"}</b>
                <small>
                  {app.loading
                    ? "Обновление…"
                    : profile
                      ? `UID ${profile.uid}`
                      : "Подключение…"}
                </small>
              </div>
            </div>
            {(app.error || app.dataError) && (
              <div className="banner error" role="alert">
                Не удалось обновить данные. {app.dataError || app.error}{" "}
                <button onClick={app.reload}>Повторить</button>
              </div>
            )}
            {!app.connected && data && (
              <div className="banner">
                Связь с приложением восстанавливается. Показаны последние
                загруженные данные; их актуальность пока не подтверждена.
              </div>
            )}
            {catalogError && (
              <div className="banner">
                Справочник названий недоступен. Предметы показаны по ID.
              </div>
            )}
            {!app.state ? (
              <div className="empty">Подключаемся к локальному приложению…</div>
            ) : tab === "catalog" ? null : !profile ? (
              <div className="empty">
                <h2>Профилей пока нет</h2>
                <p>
                  Запустите захват и войдите в игру, чтобы сохранить первый
                  снимок.
                </p>
              </div>
            ) : !data ? (
              <div className="empty">Загружаем профиль…</div>
            ) : (
              <>
                {(tab === "inventory" || tab === "characters") && (
                  <>
                    <section className="panel">
                      {tab === "inventory" && (
                        <div
                          className="categories"
                          role="group"
                          aria-label="Категории инвентаря"
                        >
                          {[
                            ...inventoryCategories,
                            ...(inventoryDeveloper
                              ? developerInventoryCategories
                              : []),
                          ].map(([c, label]) => (
                            <button
                              key={c}
                              className={category === c ? "selected" : ""}
                              onClick={() => setCategory(c)}
                              aria-pressed={category === c}
                            >
                              {label}
                              <span>{inventory[c]?.length ?? 0}</span>
                            </button>
                          ))}
                        </div>
                      )}
                      <div className="toolbar">
                        <label className="search">
                          <span>⌕</span>
                          <input
                            aria-label="Поиск"
                            placeholder={
                              tab === "characters"
                                ? "Имя персонажа или ID…"
                                : "Название, ID или GUID…"
                            }
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                          />
                          {search && (
                            <button
                              aria-label="Очистить поиск"
                              onClick={() => setSearch("")}
                            >
                              ×
                            </button>
                          )}
                        </label>
                        <select
                          aria-label="Сортировка"
                          value={tab === "characters" && sort === "count" ? "name" : sort}
                          onChange={(e) => setSort(e.target.value)}
                        >
                          <option value="name">По названию</option>
                          <option value="game">
                            {tab === "inventory"
                              ? "Порядок по справочнику игры"
                              : "По имени персонажа"}
                          </option>
                          {tab === "inventory" && <option value="count">По количеству</option>}
                          <option value="level">По уровню</option>
                        </select>
                        {tab === "inventory" && (
                          <>
                            <select
                              aria-label="Тип предмета"
                              value={subtype}
                              onChange={(e) =>
                                setInventoryFilter("subtype", e.target.value)
                              }
                            >
                              <option value="">Все типы</option>
                              {subtype &&
                                !subtypeOptions(
                                  rawRows,
                                  catalog.items ?? {},
                                ).some(([type]) => type === subtype) && (
                                  <option value={subtype}>{subtype} · 0</option>
                                )}
                              {subtypeOptions(rawRows, catalog.items ?? {}).map(
                                ([type, count]) => (
                                  <option key={type} value={type}>
                                    {type} · {count}
                                  </option>
                                ),
                              )}
                            </select>
                            <select
                              aria-label="Редкость"
                              value={rarityFilter}
                              onChange={(e) =>
                                setInventoryFilter("rarity", e.target.value)
                              }
                            >
                              <option value="">Любая редкость</option>
                              {[5, 4, 3, 2, 1, 0].map((r) => (
                                <option key={r} value={String(r)}>
                                  {r ? `${r} ★` : "Без редкости"}
                                </option>
                              ))}
                            </select>
                            {(subtype || rarityFilter) && (
                              <button
                                className="text-button"
                                onClick={() =>
                                  setInventoryFilters((all) => ({
                                    ...all,
                                    [category]: {},
                                  }))
                                }
                              >
                                Сбросить фильтры
                              </button>
                            )}
                            <div
                              className="view-toggle"
                              role="group"
                              aria-label="Вид инвентаря"
                            >
                              <button
                                aria-pressed={inventoryView === "grid"}
                                className={
                                  inventoryView === "grid" ? "selected" : ""
                                }
                                onClick={() => setInventoryView("grid")}
                              >
                                ▦ Сетка
                              </button>
                              <button
                                aria-pressed={inventoryView === "list"}
                                className={
                                  inventoryView === "list" ? "selected" : ""
                                }
                                onClick={() => setInventoryView("list")}
                              >
                                ☷ Список
                              </button>
                            </div>
                          </>
                        )}
                        {tab === "inventory" && (
                          <label className="inventory-dev-toggle">
                            <input
                              type="checkbox"
                              checked={inventoryDeveloper}
                              onChange={(e) => {
                                setInventoryDeveloper(e.target.checked);
                                if (
                                  !e.target.checked &&
                                  category === "non_inventory"
                                )
                                  setCategory("development");
                              }}
                            />
                            Режим разработчика
                          </label>
                        )}
                        <span className="muted">{tab === "characters" ? `Получено: ${gallery.filter((r:any)=>!r.unowned).length}` : `Найдено: ${rows.length}`}</span>
                      </div>
                      {tab === "characters" && <div className="character-element-filters" role="group" aria-label="Фильтр по стихиям">
                        <button aria-pressed={!selectedElements.length} onClick={()=>setSelectedElements([])}>Все стихии</button>
                        {Object.entries(elements).map(([key,[label]])=><button key={key} aria-pressed={selectedElements.includes(key)} onClick={()=>setSelectedElements(all=>all.includes(key)?all.filter(e=>e!==key):[...all,key])}>{label}</button>)}
                      </div>}
                      <div className="domain-line">
                        <span
                          className={
                            "dot " +
                            (domain?.freshness === "synchronized"
                              ? "green"
                              : domain?.freshness === "error"
                                ? "red"
                                : "amber")
                          }
                        />
                        {domain
                          ? (freshnessNames[domain.freshness] ??
                            domain.freshness)
                          : "Нет снимка для этого раздела"}
                        <span className="muted">
                          {domain
                            ? `Обновлено ${date(domain.observed_unix_ms)}`
                            : ""}
                        </span>
                        {domain?.freshness === "stale" && (
                          <b>Нужен свежий снимок</b>
                        )}
                      </div>
                      {domain?.scope !== "full" && (
                        <div className="coverage">
                          Показаны наблюдавшиеся записи. Отсутствие предмета в
                          неполном снимке не подтверждает нулевое количество.
                        </div>
                      )}
                      <div ref={virtual.ref} className="virtual-items">
                      <div style={{height:virtual.top}} aria-hidden="true" />
                      {rows.length === 0 ? (
                        <div className="empty">
                          <h2>
                            {search || subtype || rarityFilter
                              ? "Ничего не найдено"
                              : "В этом разделе пока пусто"}
                          </h2>
                          <p>
                            {search || subtype || rarityFilter
                              ? "Измените поиск или сбросьте фильтры."
                              : "Данные появятся после подходящего снимка."}
                          </p>
                        </div>
                      ) : tab === "characters" ? (
                        <CharacterGrid rows={shown} catalog={catalog} name={name} favorites={favorites} toggleFavorite={toggleFavorite} onSelect={setDetail} />
                      ) : inventoryView === "grid" ? (
                        <InventoryGrid
                          rows={shown}
                          items={catalog.items ?? {}}
                          rarities={catalog.rarities}
                          characters={data.collections.characters ?? []}
                          avatarIcons={catalog.avatarIcons ?? {}}
                          name={name}
                          onSelect={setDetail}
                        />
                      ) : (
                        <div className="table-wrap">
                          <table>
                            <thead>
                              <tr>
                                <th>Предмет</th>
                                <th>
                                  {!["weapons", "artifacts"].includes(category)
                                    ? "Количество"
                                    : category === "weapons"
                                      ? "Уровень / количество"
                                      : "Уровень"}
                                </th>
                                <th>
                                  {category === "weapons"
                                    ? "Пробуждение / редкость"
                                    : "Редкость"}
                                </th>
                                <th>Статус</th>
                                <th />
                              </tr>
                            </thead>
                            <tbody>
                              {shown.map((r) => (
                                <tr key={r.entity_key}>
                                  <td>
                                    <button
                                      className="item-name"
                                      onClick={() => setDetail(r)}
                                    >
                                      <ItemIcon
                                        fallback={name(r.values).slice(0,1)}
                                        icon={
                                          catalog.items?.[
                                            String(r.values.item_id)
                                          ]?.icon
                                        }
                                        rarity={
                                          catalog.rarities[
                                            String(r.values.item_id)
                                          ] ?? 0
                                        }
                                      />
                                      <span>
                                        {name(r.values)}
                                        <small>ID {r.values.item_id}</small>
                                      </span>
                                    </button>
                                  </td>
                                  <td className="number">
                                    {category === "weapons"
                                      ? r.source_collection === "weapons"
                                        ? `Ур. ${fmt(r.values.level)}`
                                        : `${fmt(r.values.count)} шт.`
                                      : fmt(
                                          category === "artifacts"
                                            ? r.values.level
                                            : r.values.count,
                                        )}
                                  </td>
                                  <td className="stars">
                                    {r.source_collection === "weapons"
                                      ? `R${r.values.refinement ?? "—"}`
                                      : "★".repeat(
                                          catalog.rarities[
                                            String(r.values.item_id)
                                          ] ?? 0,
                                        ) || "—"}
                                  </td>
                                  <td>
                                    <span className="subtle">
                                      {r.confirmation === "observed"
                                        ? "Подтверждён"
                                        : "Из прошлых данных"}
                                      {r.values.locked ? " · 🔒" : ""}
                                    </span>
                                  </td>
                                  <td>
                                    <button
                                      className="text-button"
                                      onClick={() => setDetail(r)}
                                    >
                                      Подробнее ↗
                                    </button>
                                  </td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </div>
                      )}
                      <div style={{height:virtual.bottom}} aria-hidden="true" />
                      </div>

                    </section>
                    {tab === "inventory" && (
                      <p className="footnote">
                        Игровые изображения © HoYoverse. Источники:{" "}
                        <a
                          href="https://enka.network/"
                          target="_blank"
                          rel="noreferrer"
                        >
                          Enka.Network
                        </a>{" "}
                        и{" "}
                        <a
                          href="https://github.com/PathOfGenshin/resources"
                          target="_blank"
                          rel="noreferrer"
                        >
                          PathOfGenshin
                        </a>
                        . Загруженные иконки сохраняются на этом компьютере.
                      </p>
                    )}
                    {tab === "characters" && (
                      <p className="footnote">
                        Итоговые характеристики отражают последние полученные от
                        сервера значения.
                      </p>
                    )}
                  </>
                )}
                {tab === "history" && (
                  <History
                    key={app.selected}
                    profile={app.selected}
                    tick={app.tick}
                    catalog={catalog}
                    name={name}
                  />
                )}
                {tab === "profile" && (
                  <PlayerProfile
                    data={data}
                    profile={profile}
                    catalog={catalog}
                    openCharacter={(guid)=>{const row=roster.find(r=>r.entity_key===guid);if(row)setDetail(row);}}
                    avatarName={(id) => name({ avatar_id: id })}
                  />
                )}
                {tab === "teams" && (
                  <Teams
                    data={data}
                    avatarIcons={catalog.avatarIcons ?? {}}
                    avatarName={(id) => name({ avatar_id: id })}
                    openCharacter={(guid) => {
                      const row = roster.find((r) => r.entity_key === guid);
                      if (row) setDetail(row);
                    }}
                  />
                )}
                {tab === "wishes" && <Wishes key={app.selected} profile={app.selected} tick={app.tick} catalog={catalog} />}
                {tab === "planner" && <Planner key={app.selected} profile={app.selected} tick={app.tick} data={data} catalog={catalog} />}
                {tab === "achievements" && (
                  <Achievements data={data} catalog={catalog} />
                )}
                {tab === "captures" && (
                  <Captures
                    key={app.selected}
                    profile={app.selected}
                    tick={app.tick}
                    data={data}
                    name={name}
                  />
                )}
                {tab === "captures" && (
                  <>
                <section className="capture-help">
                  <div>
                    <b>Захват игровой сессии</b>
                    <p>
                      Запускайте до входа в игру из PowerShell администратора.
                      Остановка — Ctrl+C или выход из игры.
                    </p>
                  </div>
                  <button
                    onClick={() =>
                      navigator.clipboard
                        .writeText(
                          "powershell.exe -NoProfile -ExecutionPolicy RemoteSigned -File .\\scripts\\Start-RealtimeCapture.ps1 -RecordRaw",
                        )
                        .then(() => setCopied(true))
                        .catch(() => setCopyError(true))
                    }
                  >
                    Скопировать команду
                  </button>
                </section>
                {copied && (
                  <p className="footnote" role="status">
                    Команда скопирована. Выполните её из папки Vision Overlay.
                  </p>
                )}
                {copyError && (
                  <pre>
                    powershell.exe -NoProfile -ExecutionPolicy RemoteSigned
                    -File .\scripts\Start-RealtimeCapture.ps1 -RecordRaw
                  </pre>
                )}
                  </>
                )}
              </>
            )}
            {tab === "backups" && (
              <Backups selected={app.selected} catalog={catalog} />
            )}
            {tab === "maintenance" && <Maintenance />}
            {tab === "catalog" && (
              <CatalogPanel onChanged={() => setCatalogTick((n) => n + 1)} />
            )}
            {tab === "captures" && <CaptureAutomation />}
            <footer>
              VISION OVERLAY{" "}
              <span>
                Локальное приложение · Данные хранятся на этом компьютере
              </span>
            </footer>
          </div>
        </main>
        {detail && data && (
          <Details
            key={detail.entity_key}
            row={detail}
            data={data}
            catalog={catalog}
            name={name}
            close={() => {setDetail(null);setDetailParent(null);}}
            openItem={(item)=>{setDetailParent(detail);setDetail(item);}}
            back={detailParent?()=>{setDetail(detailParent);setDetailParent(null);}:undefined}
          />
        )}
      </div>
    </CatalogIconVersion.Provider>
  );
  function setCopied(v: boolean) {
    setCopyState(v);
  }
  function setCopyError(v: boolean) {
    setCopyErrorState(v);
  }
  // Kept outside profile requests so copying a command never changes game state.
}
function Stat({
  label,
  value,
  symbol,
}: {
  label: string;
  value: string;
  symbol: string;
}) {
  return (
    <div className="header-stat">
      <span className="header-stat-symbol" aria-hidden="true">{symbol}</span>
      <div>
        <small>{label}</small>
        <strong>{value}</strong>
      </div>
    </div>
  );
}

function Details({
  row,
  data,
  catalog,
  name,
  close,
  openItem,
  back,
}: {
  row: Row;
  data: ProfileData;
  catalog: Catalog;
  name: (v: any) => string;
  close: () => void;
  openItem: (item:Row)=>void;
  back?: ()=>void;
}) {
  const drawer = useRef<HTMLElement>(null),
    onClose = useRef(close);
  onClose.current = close;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const f = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose.current();
      if (e.key === "Tab" && drawer.current) {
        const nodes = Array.from(
          drawer.current.querySelectorAll<HTMLElement>(
            'button,summary,a[href],input,select,[tabindex="0"]',
          ),
        ).filter((n) => n.getClientRects().length > 0);
        const first = nodes[0],
          last = nodes.at(-1);
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last?.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first?.focus();
        }
      }
    };
    window.addEventListener("keydown", f);
    return () => {
      document.body.style.overflow = overflow;
      window.removeEventListener("keydown", f);
      previous?.focus();
    };
  }, []);
  const live =
      (row.guids
        ? aggregate(data.collections.material_stacks ?? []).find(
            (r: Row) => r.entity_key === row.entity_key,
          )
        : Object.values(data.collections)
            .flat()
            .find((r) => r.entity_key === row.entity_key)) ?? row,
    v = live.values,
    isCharacter = !!v.avatar_id;
  const referenceOnly=!!(row as Row & {unowned?:boolean}).unowned;
  const owner = !isCharacter && ["weapons","artifacts"].includes(live.source_collection ?? row.source_collection ?? "") ? equipmentOwners(data.collections.characters).get(live.entity_key) : null;
  const artifactInfo=artifactStats(v,catalog);
  return (
    <div className="drawer-backdrop" onClick={close}>
      <aside
        ref={drawer}
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-label={name(v)}
        onClick={(e) => e.stopPropagation()}
      >
        <button
          className="close"
          onClick={close}
          autoFocus
          aria-label="Закрыть карточку"
        >
          ×
        </button>
        {back&&<button className="detail-back" onClick={back}>‹ К персонажу</button>}
        <div className="eyebrow">{isCharacter ? "ПЕРСОНАЖ" : "ПРЕДМЕТ"}</div>
        <h2>{name(v)}</h2>
        <div className="detail-icon">
          <ItemIcon
            icon={
              isCharacter
                ? catalog.avatarIcons?.[String(v.avatar_id)]
                : catalog.items?.[String(v.item_id)]?.icon
            }
            rarity={catalog.rarities[String(v.item_id)] ?? 0}
            fallback={name(v).slice(0,1)}
            large
          />
        </div>
        <p className="muted">{isCharacter ? referenceOnly ? "Не получен" : `Дружба ${v.friendship?.level ?? "—"}` : `ID ${v.item_id}`}</p>
        {owner && <div className="detail-equipment-owner"><ItemIcon icon={catalog.avatarIcons?.[owner.values.avatar_id]} fallback={name(owner.values).slice(0,1)} /><div><small>Экипировано</small><strong>{name(owner.values)}</strong></div></div>}
        {isCharacter &&
          catalog.avatarMetadata?.[String(v.avatar_id)]?.weaponType ===
            "WEAPON_CROSSBOW" && (
            <p className="footnote">
              Отдельная запись персонажа игрового режима с винтовками. Она
              сохраняется в профиле отдельно от обычного Путешественника.
            </p>
          )}
        {!referenceOnly && <div className="detail-summary">
          {isCharacter
            ? `Уровень ${characterLevel(v)}`
            : v.kind === "weapon" ? `Уровень ${v.level ?? "—"} · Возвышение ${v.ascension ?? "—"}`
            : `Количество ${fmt(v.count)}${v.level !== undefined ? " · Уровень " + v.level : ""}${v.refinement !== undefined ? " · R" + v.refinement : ""}`}
        </div>}
        {isTraveler(v) && (
          <p className="muted">
            Активная стихия: {elementName(v.skill_depot_id, catalog)}
          </p>
        )}
        {isCharacter ? <>
          {!referenceOnly&&characterLevel(v)!==90&&<NextUpgrades values={v} catalog={catalog} data={data} only="progress"/>}
          <CharacterDetails values={v} catalog={catalog} data={data} name={name} openItem={openItem} referenceOnly={referenceOnly}/>
          {!referenceOnly&&<TravelerSets values={v} catalog={catalog} data={data}/>}
        </> : v.kind === "weapon" ? <>
          <WeaponDetails values={v} catalog={catalog}/>
          {v.level<90&&<NextUpgrades values={v} catalog={catalog} data={data} only="progress"/>}
        </> : <NextUpgrades values={v} catalog={catalog} data={data}/>}
        {v.kind === "artifact" && (
          <>
            <h3>Основная характеристика</h3>
            <p>
              {artifactInfo.main
                ? `${statName(artifactInfo.main.type)} · ${artifactInfo.main.value !== null ? statValue(artifactInfo.main.type, artifactInfo.main.value) : "значение не определено"}`
                : `Не определена (ID ${v.main_prop_id})`}
            </p>
            <h3>Дополнительные характеристики</h3>
            <dl>
              {artifactInfo.substats.map(({type, value}:any) => (
                <React.Fragment key={type}>
                  <dt>{statName(type)}</dt>
                  <dd>{statValue(type, value)}</dd>
                </React.Fragment>
              ))}
            </dl>
          </>
        )}
        {v.kind === "artifact"&&<ArtifactSetDetails values={v} catalog={catalog}/>}
        {!referenceOnly && <ProgressionReference values={v} catalog={catalog} />}
        {!referenceOnly && <details>
          <summary>Диагностические данные</summary>
          <p>
            GUID {live.entity_key} · {live.value_format}
          </p>
          <pre>{JSON.stringify(v, null, 2)}</pre>
        </details>}
      </aside>
    </div>
  );
}

function History({
  profile,
  tick,
  catalog,
  name,
}: {
  profile: string;
  tick: number;
  catalog: Catalog;
  name: (v: any) => string;
}) {
  const [items, setItems] = useState<any[]>([]),
    [cursor, setCursor] = useState<string | null>(null),
    [error, setError] = useState(""),
    [search, setSearch] = useState(""),
    [busy, setBusy] = useState(false);
  const generation = useRef(0);
  useEffect(() => {
    const controller = new AbortController(),
      g = ++generation.current;
    setBusy(true);
    get(
      `/api/v1/profiles/${profile}/events?order=desc&limit=1000`,
      controller.signal,
    )
      .then((r) => {
        if (g !== generation.current) return;
        setItems(r.items);
        setCursor(r.next_cursor);
        setError("");
        setBusy(false);
      })
      .catch((e) => {
        if (!controller.signal.aborted) {
          setError(String(e));
          setBusy(false);
        }
      });
    return () => controller.abort();
  }, [profile, tick]);
  function text(e: any) {
    if (e.kind === "state.batch")
      return (e.data?.operations ?? [])
        .map(
          (o: any) =>
            `${labels[o.collection] ?? o.collection}: ${describeOperation(o, name)}`,
        )
        .join("; ");
    if (e.kind === "material_changed") {
      const c = e.data?.change;
      return c ? `${name(c)} · ${c.previous_count} → ${c.count}` : e.kind;
    }
    return (
      (
        {
          currency_changed: "Изменение валюты",
          character_progress_changed: "Повышение уровня персонажа",
          talent_changed: "Улучшение таланта",
          equipment_changed: "Смена экипировки",
          item_changed: "Изменение экипировки",
          friendship_changed: "Изменение дружбы",
          achievement_changed: "Изменение достижения",
          constellation_changed: "Активация созвездия",
          traveler_element_changed: "Обновление стихии Путешественника",
          "state.invalidated": "Разрыв данных",
          "capture.closed": "Захват завершён",
          "identity.confirmed": "Профиль подтверждён",
          "state.snapshot": "Получен снимок",
        } as Record<string, string>
      )[e.kind] ?? e.kind
    );
  }
  const visible = items
    .filter(
      (e) =>
        e.kind !== "diagnostic.notice" &&
        (!search ||
          `${text(e)} ${e.kind}`.toLowerCase().includes(search.toLowerCase())),
    )
    .slice();
  return (
    <section className="panel">
      <div className="toolbar">
        <input
          aria-label="Поиск по истории"
          placeholder="Поиск по событиям…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <span className="muted">
          {busy ? "Обновление…" : `${visible.length} событий`}
        </span>
      </div>
      {error && (
        <div role="alert" className="banner error">
          {error}
        </div>
      )}
      <div className="coverage">
        Показаны события, сохранённые приложением. Время указано по часовому
        поясу компьютера.
        {cursor
          ? " Показаны последние события; более ранние записи можно загрузить ниже."
          : ""}
      </div>
      {visible.length === 0 ? (
        <div className="empty">Подходящих событий пока нет</div>
      ) : (
        <div className="timeline">
          {visible.map((e) => (
            <details key={e.revision}>
              <summary>
                <time>{date(e.observed_unix_ms)}</time>
                <span>
                  {e.projection_result === "rejected" ? "⚠ Отклонено · " : ""}
                  {text(e)}
                </span>
                <small>#{e.revision}</small>
              </summary>
              <pre>
                {JSON.stringify(
                  e.data ?? {
                    kind: e.kind,
                    snapshot_payload_omitted: e.snapshot_payload_omitted,
                  },
                  null,
                  2,
                )}
              </pre>
            </details>
          ))}
        </div>
      )}
      {cursor && (
        <button
          className="load-more"
          disabled={busy}
          onClick={async () => {
            const g = generation.current;
            setBusy(true);
            try {
              const r = await get(
                `/api/v1/profiles/${profile}/events?order=desc&limit=1000&cursor=${encodeURIComponent(cursor)}`,
              );
              if (g !== generation.current) return;
              setItems((old) => [...old, ...r.items]);
              setCursor(r.next_cursor);
            } catch (e) {
              setError(String(e));
            } finally {
              if (g === generation.current) setBusy(false);
            }
          }}
        >
          Загрузить следующие записи
        </button>
      )}
    </section>
  );
}

function Achievements({
  data,
  catalog,
}: {
  data: ProfileData;
  catalog: Catalog;
}) {
  const [search, setSearch] = useState(""),
    [status, setStatus] = useState("all"),
    [category, setCategory] = useState("all"),
    [limit, setLimit] = useState(80);
  const all = data.collections.achievements ?? [];
  const grouped = useMemo(
    () => achievementGroups(all, catalog.achievements),
    [all, catalog.achievements],
  );
  const categories = [
    ...new Map(
      grouped.map((g) => [String(g.meta.goal_id ?? "unknown"), g.meta]),
    ).entries(),
  ];
  const rows = grouped.filter((g) =>
    achievementMatches(g, catalog.achievements, status, category, search),
  );
  const claimed = all.filter(
    (r) =>
      !catalog.achievements[r.entity_key]?.excluded &&
      r.values.status_code === 3,
  ).length;
  useEffect(() => setLimit(80), [search, status, category]);
  const stageStatus = (row?: Row) =>
    !row
      ? "Нет данных"
      : row.values.status_code === 3
        ? "Награда получена"
        : row.values.status_code === 2
          ? "Награда доступна"
          : `${row.values.current_progress ?? "?"} / ${row.values.total_progress ?? "?"}`;
  return (
    <section className="panel achievements-panel">
      <div className="toolbar">
        <input
          aria-label="Поиск достижений"
          placeholder="Название, описание или ID…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <select
          aria-label="Статус достижения"
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        >
          <option value="all">Все статусы</option>
          <option value="unearned">Не получены</option>
          <option value="claimed">Награда получена</option>
          <option value="finished">Выполнены, награда не получена</option>
        </select>
        <select
          aria-label="Категория достижений"
          value={category}
          onChange={(e) => setCategory(e.target.value)}
        >
          <option value="all">Все категории</option>
          {categories.map(([id, meta]) => (
            <option key={id} value={id}>
              {meta.category ?? "Без категории"}
            </option>
          ))}
        </select>
      </div>
      <div className="domain-line">
        Найдено: {rows.length} · Награда получена: {claimed}{" "}
        <span className="subtle">
          Счётчик наград учитывает каждый этап отдельно
        </span>
      </div>
      <div className="achievement-list">
        {rows.slice(0, limit).map((group, index) => (
          <React.Fragment key={group.key}>
            {(index === 0 ||
              rows[index - 1].meta.goal_id !== group.meta.goal_id) && (
              <h2 className="achievement-category">
                {group.meta.category ?? "Без категории"}
              </h2>
            )}
            <article className="achievement-card">
              <span className="achievement-symbol">✧</span>
              <div className="achievement-body">
                <div className="achievement-title">
                  <h3>{group.meta.name ?? `Достижение ${group.key}`}</h3>
                  {group.levels.length > 1 && (
                    <div
                      className="achievement-stars"
                      aria-label="Этапы достижения"
                    >
                      {group.levels.map((level: any) => (
                        <span
                          key={level.id}
                          title={`Этап ${level.meta.stage ?? 1}: ${stageStatus(level.row)}`}
                          aria-label={`Этап ${level.meta.stage ?? 1}: ${stageStatus(level.row)}`}
                          className={
                            level.row?.values.status_code >= 2 ? "earned" : ""
                          }
                        >
                          {level.row?.values.status_code >= 2 ? "★" : "☆"}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
                <div className="achievement-stages">
                  {group.levels.map((level: any) => (
                    <div key={level.id} className="achievement-stage">
                      <div>
                        {group.levels.length > 1 && (
                          <small className="stage-number">
                            Этап {level.meta.stage ?? 1}
                          </small>
                        )}
                        <p>{level.meta.description ?? "Описание недоступно"}</p>
                        <small>ID {level.id}</small>
                      </div>
                      <span className="subtle">{stageStatus(level.row)}</span>
                    </div>
                  ))}
                </div>
              </div>
            </article>
          </React.Fragment>
        ))}
      </div>
      {!rows.length && (
        <div className="empty">Достижений по этим условиям нет</div>
      )}
      {rows.length > limit && (
        <button className="load-more" onClick={() => setLimit((n) => n + 80)}>
          Показать ещё
        </button>
      )}
    </section>
  );
}

function Captures({
  profile,
  tick,
  data,
  name,
}: {
  profile: string;
  tick: number;
  data: ProfileData;
  name: (v: any) => string;
}) {
  const requestedCapture = useRef("");
  const [captures, setCaptures] = useState<any[]>([]),
    [capture, setCapture] = useState(""),
    [snapshots, setSnapshots] = useState<any[]>([]),
    [snapshot, setSnapshot] = useState<any>(null),
    [error, setError] = useState("");
  useEffect(() => {
    const c = new AbortController();
    pages("/api/v1/captures", c.signal)
      .then((v) =>
        setCaptures(
          v.sort((a, b) => b.imported_at.localeCompare(a.imported_at)),
        ),
      )
      .catch((e) => {
        if (!c.signal.aborted) setError(String(e));
      });
    return () => c.abort();
  }, [tick]);
  useEffect(() => {
    setSnapshot(null);
    setSnapshots([]);
    if (!capture) return;
    const c = new AbortController();
    pages(`/api/v1/captures/${capture}/snapshots`, c.signal)
      .then((v) => setSnapshots(v.filter((s) => s.profile_id === profile)))
      .catch((e) => {
        if (!c.signal.aborted) setError(String(e));
      });
    return () => c.abort();
  }, [capture, profile]);
  requestedCapture.current = capture;
  const selected = captures.find((c) => c.capture_id === capture);
  const diff = snapshot
    ? compareEntities(
        snapshot.entities,
        Object.fromEntries(
          (data.collections[snapshot.collection] ?? [])
            .filter((r) => r.confirmation === "observed")
            .map((r) => [r.entity_key, r.values]),
        ),
      )
    : [];
  return (
    <section className="panel">
      <div className="toolbar">
        <select
          aria-label="Захват"
          value={capture}
          onChange={(e) => setCapture(e.target.value)}
        >
          <option value="">Выберите захват…</option>
          {captures.map((c) => (
            <option value={c.capture_id} key={c.capture_id}>
              {new Date(c.imported_at).toLocaleString("ru-RU")} ·{" "}
              {c.terminal_status} · {c.event_count} событий
            </option>
          ))}
        </select>
      </div>
      {error && (
        <div role="alert" className="banner error">
          {error}
        </div>
      )}
      {selected && (
        <div className="domain-line">
          {selected.terminal_status} · {selected.event_count} событий ·{" "}
          {selected.capture_id}
        </div>
      )}
      <DiagnosticExport capture={capture}/>
      {capture && snapshots.length === 0 && (
        <div className="empty">
          Снимков выбранного профиля в этом захвате нет
        </div>
      )}
      <div className="snapshot-list">
        {snapshots.map((s) => (
          <button
            className={
              snapshot?.snapshot_id === s.snapshot_id ? "selected" : ""
            }
            key={s.snapshot_id}
            onClick={() => {
              get("/api/v1/snapshots/" + s.snapshot_id)
                .then((v) => {
                  if (requestedCapture.current === s.capture_id) setSnapshot(v);
                })
                .catch((e) => setError(String(e)));
            }}
          >
            {labels[s.collection] ?? s.collection}
            <small>
              Событие {s.source_sequence} ·{" "}
              {s.scope === "full" ? "Полный снимок" : "Частичное покрытие"}
            </small>
          </button>
        ))}
      </div>
      {snapshot && (
        <div className="comparison">
          <h3>
            {labels[snapshot.collection] ?? snapshot.collection}: сравнение с
            текущим состоянием
          </h3>
          <p>
            {Object.keys(snapshot.entities).length} записей в снимке ·{" "}
            {diff.length} различий
          </p>
          <div className="coverage">
            Отсутствие записи в частичном снимке означает «не наблюдалась».
            Сравнение не меняет выбранный профиль или его данные.
          </div>
          {diff.slice(0, 100).map((d: any) => (
            <div className="difference" key={d.key}>
              <span>
                {name(d.after ?? d.before)}
                <small>GUID / ключ {d.key}</small>
              </span>
              <b>
                {d.before?.count !== undefined || d.after?.count !== undefined
                  ? `${d.before?.count ?? "не наблюдался"} → ${d.after?.count ?? "не наблюдался"}`
                  : d.kind === "added"
                    ? "Новая запись"
                    : d.kind === "removed"
                      ? "Нет в текущей базе"
                      : "Изменены характеристики"}
              </b>
            </div>
          ))}
          {diff.length > 100 && <p>Показаны первые 100 различий.</p>}
          <details>
            <summary>Полные данные исходного снимка</summary>
            <pre>{JSON.stringify(snapshot.entities, null, 2)}</pre>
          </details>
        </div>
      )}
    </section>
  );
}
function CaptureAutomation() {
  const [status, setStatus] = useState<any>(null),
    [error, setError] = useState(false),
    [copied, setCopied] = useState(false),
    [pending, setPending] = useState<{
      id: string;
      action: string;
      at: number;
      instance: string;
    } | null>(null),
    [feedback, setFeedback] = useState("");
  const pendingRef = useRef(pending);
  pendingRef.current = pending;
  const errors: Record<string, string> = {
    helper_unavailable: "Наблюдатель не отвечает. Проверьте его запуск.",
    helper_upgrade_required: "Наблюдатель нужно обновить и перезапустить.",
    already_running: "Захват уже запущен.",
    run_changed: "Захват уже завершился или сменился. Обновите статус.",
    command_pending: "Предыдущая команда ещё обрабатывается.",
    helper_restarted: "Наблюдатель перезапустился. Повторите команду.",
    expired_command: "Команда не была обработана вовремя.",
    launch_failed: "Не удалось запустить захват. Проверьте журнал наблюдателя.",
    capture_failed: "Захват завершился с ошибкой. Проверьте журнал.",
    invalid_command: "Команда отклонена наблюдателем.",
    observation_failed: "Не удалось проверить процессы игры.",
  };
  useEffect(() => {
    let disposed = false,
      inflight = false;
    const controller = new AbortController();
    async function refresh() {
      if (inflight) return;
      inflight = true;
      try {
        const value = await get(
          "/api/v1/capture/automation",
          controller.signal,
        );
        if (disposed) return;
        setStatus(value);
        setError(false);
        const p = pendingRef.current;
        if (p) {
          const result = value.last_command;
          if (value.watcher_instance && value.watcher_instance !== p.instance) {
            setFeedback(
              "Наблюдатель перезапустился. Проверьте состояние захвата перед повторной командой.",
            );
            setPending(null);
            pendingRef.current = null;
          } else if (
            result?.request_id === p.id &&
            (result.outcome === "completed" ||
              result.outcome === "rejected" ||
              result.outcome === "failed")
          ) {
            setFeedback(
              result.outcome === "completed"
                ? p.action === "start"
                  ? "Источник готов. Для синхронизации дождитесь снимка после входа в игру."
                  : "Захват завершён. Запись финализирована; бэкенд завершает доставку оставшихся событий."
                : (errors[result.code] ??
                    "Команда не выполнена: " + result.code),
            );
            setPending(null);
            pendingRef.current = null;
          } else if (Date.now() - p.at > 45000) {
            setFeedback(
              "Подтверждение команды не получено. Проверьте состояние и журнал наблюдателя.",
            );
            setPending(null);
            pendingRef.current = null;
          }
        }
      } catch {
        if (!disposed) setError(true);
      } finally {
        inflight = false;
      }
    }
    void refresh();
    const timer = setInterval(refresh, 1000);
    return () => {
      disposed = true;
      controller.abort();
      clearInterval(timer);
    };
  }, []);
  async function control(action: "start" | "stop") {
    if (pendingRef.current || !status) return;
    const p = {
      id: crypto.randomUUID(),
      action,
      at: Date.now(),
      instance: status.watcher_instance,
    };
    pendingRef.current = p;
    setPending(p);
    setFeedback("");
    try {
      const response = await fetch("/api/v1/capture/control", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-Vision-Control": "1",
        },
        body: JSON.stringify({
          request_id: p.id,
          action,
          target_run_id: action === "stop" ? status.capture_run_id : null,
        }),
      });
      const result = await response.json();
      if (!response.ok) {
        setFeedback(
          errors[result.error?.code] ??
            "Команда не выполнена: " + (result.error?.code ?? response.status),
        );
        setPending(null);
        pendingRef.current = null;
      }
    } catch {
      setFeedback("Ответ сервера не получен; уточняем состояние команды…");
    }
  }
  const titles: Record<string, string> = {
    not_configured: "Не настроен",
    disabled: "Отключён",
    idle: "Ручной режим · захват не запущен",
    watcher_unavailable: "Наблюдатель не отвечает",
    waiting: "Захват не запущен",
    capturing: "Захват работает",
    capture_started: "Запуск захвата",
    capture_finished: "Захват завершён",
    capture_failed: "Ошибка захвата",
    launch_failed: "Ошибка запуска",
    already_running: "Ожидание нового входа в игру",
    external_capture: "Работает внешний захват",
    handled: "Захват не запущен",
    observation_failed: "Не удалось проверить процесс игры",
    starting: "Запуск источника…",
    ready: "Захват работает · источник готов",
    stopping: "Завершение записи…",
  };
  const phase = status?.capture_state ?? status?.state;
  const available =
    !!status?.control_available &&
    !error &&
    !pending &&
    !status?.command_pending;
  const command = status?.enabled
    ? (status?.disable_command ?? "powershell.exe -NoProfile -ExecutionPolicy RemoteSigned -File .\\scripts\\Configure-AutoCapture.ps1 -Disable")
    : (status?.installation_command ?? "powershell.exe -NoProfile -ExecutionPolicy RemoteSigned -File .\\scripts\\Configure-AutoCapture.ps1");
  return (
    <section className="automation">
      <div className="automation-heading">
        <div>
          <b>Управление захватом</b>
          <p>
            <span
              className={
                "dot " + (status?.control_available ? "green" : "amber")
              }
            />
            {error
              ? "Статус временно недоступен"
              : (titles[phase] ?? "Проверка состояния…")}
          </p>
        </div>
        <div className="capture-buttons">
          <button
            disabled={
              !available || !!status.capture_run_id || status.external_capture
            }
            onClick={() => void control("start")}
          >
            {pending?.action === "start" ? "Запускаем…" : "Начать захват"}
          </button>
          <button
            disabled={
              !available || !status.capture_run_id || phase === "stopping"
            }
            onClick={() => void control("stop")}
          >
            {pending?.action === "stop"
              ? "Останавливаем…"
              : "Остановить захват"}
          </button>
        </div>
      </div>
      <p>
        Запускайте захват до входа в игру и дождитесь готовности источника. При
        запуске посреди игры для получения снимка потребуется повторный вход в
        игровую сессию. Ручная остановка сохраняет запись и не запускает захват
        заново автоматически в том же процессе игры.
      </p>
      {status?.watcher_alive && !status?.control_available && (
        <p className="control-message">
          Для кнопок управления нужно обновить работающий наблюдатель.
        </p>
      )}
      {status?.external_capture && (
        <p className="control-message">
          Этот захват запущен вне помощника. Остановите его через Ctrl+C в
          исходном терминале.
        </p>
      )}
      {feedback && (
        <p className="control-message" role="status">
          {feedback}
        </p>
      )}
      <div className="automation-heading auto-settings">
        <div>
          <b>Автоматический запуск</b>
          <p>
            {status?.enabled ? "Включён" : "Отключён"} ·{" "}
            {status?.watcher_alive
              ? "наблюдатель работает"
              : "наблюдатель недоступен"}
          </p>
        </div>
        <button
          disabled={!status || error}
          onClick={() => {
            navigator.clipboard
              .writeText(command)
              .then(() => setCopied(true))
              .catch(() => setCopied(false));
          }}
        >
          {status?.enabled ? "Команда отключения" : "Команда включения"}
        </button>
      </div>
      <details>
        <summary>Настройка и ограничения</summary>
        <p>
          {status?.portable
            ? "Настройте захват через меню значка Vision Overlay в трее. Для ручной настройки можно выполнить команду ниже в PowerShell администратора."
            : "Однократно выполните команду ниже в PowerShell администратора из папки Vision Overlay."}
          {" "}Помощник работает от вашего пользователя с повышенными правами. Пароль не сохраняется.
        </p>
        <pre>{command}</pre>
        <p>
          Помощник принимает только запуск и штатную остановку захвата.
          Отключение автоматического старта оставляет ручные кнопки доступными.
          При выходе из игры захват завершится самостоятельно. Журналы:
          {status?.portable ? "data/auto-capture." : "local-data/auto-capture."}
        </p>
      </details>
      {copied && (
        <p role="status">
          Команда скопирована. Выполните её в PowerShell администратора.
        </p>
      )}
    </section>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
