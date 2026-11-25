import { gameText, language, loadGameLanguage, localizeGameData, setLanguage, tr, uiLocale } from "./i18n.mjs";
import {DiagnosticExport} from "./DiagnosticExport";
import {artifactStats} from "./equipment-details.mjs";
import { WeaponDetails, ArtifactSetDetails } from "./EquipmentDetails";
import { CharacterDetails } from "./CharacterDetails";
import { CharacterGrid, elements } from "./CharacterGrid";
import {characterGallery,characterPriority,savedFavorites,matchesCharacterElements} from "./characters.mjs";
import { equipmentOwners } from "./equipment-owners.mjs";
import {gameSession,selectedProfile} from "./profile-selection.mjs";
import {useInventoryWindow} from "./useInventoryWindow";
import { AbyssStat, TheaterStat, OnslaughtStat } from "./AbyssStat";
import { AbyssView } from "./AbyssView";
import { TheaterView } from "./TheaterView";
import { OnslaughtView } from "./OnslaughtView";
import { ResinStat } from "./ResinStat";
import { observedStock } from "./upgrades.mjs";
import { Planner } from "./PlanningView";
import { DungeonsView } from "./DungeonsView";
import { GuidesView } from "./GuidesView";
import { WorldMapView } from "./WorldMapView";
import type { GoalChoice } from "./GoalPicker";
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
  originalWishNames?: Pick<Catalog, "avatarNames" | "names">;
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
const headerRegionCodes: Record<string, string> = {
  os_euro: "EU",
  os_asia: "AS",
  os_usa: "AM",
  os_cht: "TW/HK/MO",
};
const fmt = (v: unknown) =>
  v === null || v === undefined ? "—" : Number(v).toLocaleString(uiLocale);
const date = (v: unknown) =>
  v ? new Date(Number(v)).toLocaleString(uiLocale) : tr("Время неизвестно");
const propNames: Record<string, string> = {
  HP: "HP",
  HP_PERCENT: "HP",
  ATTACK: tr("Сила атаки"),
  ATTACK_PERCENT: tr("Сила атаки"),
  DEFENSE: tr("Защита"),
  DEFENSE_PERCENT: tr("Защита"),
  CRITICAL: tr("Шанс крит. попадания"),
  CRITICAL_HURT: tr("Крит. урон"),
  CHARGE_EFFICIENCY: tr("Восст. энергии"),
  ELEMENT_MASTERY: tr("Мастерство стихий"),
  HEAL_ADD: tr("Бонус лечения"),
  FIRE_ADD_HURT: tr("Бонус Пиро"),
  WATER_ADD_HURT: tr("Бонус Гидро"),
  ICE_ADD_HURT: tr("Бонус Крио"),
  ELEC_ADD_HURT: tr("Бонус Электро"),
  WIND_ADD_HURT: tr("Бонус Анемо"),
  ROCK_ADD_HURT: tr("Бонус Гео"),
  GRASS_ADD_HURT: tr("Бонус Дендро"),
  PHYSICAL_ADD_HURT: tr("Бонус физ. урона"),
};
function statName(type: string) {
  return propNames[type.replace("FIGHT_PROP_", "")] ?? type;
}
function statValue(type: string, v: number) {
  return /PERCENT|CRITICAL|CHARGE_EFFICIENCY|ADD_HURT|HEAL_ADD/.test(type)
    ? `${(v * 100).toLocaleString(uiLocale, { maximumFractionDigits: 1 })}%`
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
function pageRoute() {
  const hash = window.location.hash;
  if (/^#guides(?:\/\d+)?$/.test(hash) ||
      hash === "#abyss" || hash === "#theater" || hash === "#onslaught" || hash === "#dungeons" || hash === "#map") return hash.slice(1);
  return null;
}

function initialTab() {
  const tab = sessionStorage.getItem("vo.language.return-tab");
  sessionStorage.removeItem("vo.language.return-tab");
  return tab ?? "profile";
}

function App() {
  const [settingsOpen,setSettingsOpen]=useState(false);
  const [linkedTab, setLinkedTab] = useState(pageRoute);
  useEffect(() => {
    const update = () => {
      const route = pageRoute();
      setLinkedTab(route);
      if (route) window.scrollTo(0, 0);
    };
    window.addEventListener("hashchange", update);
    return () => window.removeEventListener("hashchange", update);
  }, []);
  const [copied, setCopyState] = useState(false),
    [copyError, setCopyErrorState] = useState(false);
  const app = useApp(),
    [catalog, setCatalog] = useState<Catalog>(emptyCatalog),
    [catalogError, setCatalogError] = useState(false),
    [catalogTick, setCatalogTick] = useState(0),
    [baseTab, setBaseTab] = useState(initialTab),
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
  const tab = linkedTab?.split("/")[0] ?? baseTab;
  function setTab(next: string) {
    if (linkedTab) {
      window.history.replaceState(null, "", window.location.pathname + window.location.search);
      setLinkedTab(null);
    }
    setBaseTab(next);
  }
  const [goalRequest, setGoalRequest] = useState<{profile: string; choice: GoalChoice} | null>(null);
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
          setCatalog(language === "en" ? {
            ...value,
            originalWishNames: { names: value.names, avatarNames: value.avatarNames },
            names: localizeGameData(value.names),
            avatarNames: localizeGameData(value.avatarNames),
            skillNames: localizeGameData(value.skillNames),
            achievements: localizeGameData(value.achievements),
            characterDetails: localizeGameData(value.characterDetails),
            equipmentDetails: localizeGameData(value.equipmentDetails),
          } : value);
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
      ? (catalog.avatarNames[String(v.avatar_id)] ?? tr("Персонаж {0}", v.avatar_id))
      : (catalog.names[String(v.item_id)] ??
        tr("Предмет {0}", v.item_id ?? tr("без ID")));
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
        v.toLocaleLowerCase(uiLocale).includes(search.toLocaleLowerCase(uiLocale)),
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
            : name(a.values).localeCompare(name(b.values), uiLocale)),
    );
  }, [rawRows, category, tab, search, sort, catalog, subtype, rarityFilter, favoritesByProfile, app.selected, selectedElements]);
  const windowKey=JSON.stringify([app.selected,tab,category,search,sort,subtype,rarityFilter,inventoryView,selectedElements]);
  const virtual=useInventoryWindow(rows,windowKey);
  const shown=virtual.shown,
    domain =
      tab === "characters"
        ? data?.domains.find((d) => d.collection === "characters")
        : categoryDomain(category, rawRows, data?.domains ?? []);
  const resin = data?.collections.wallet?.find(row => row.entity_key === "106");
  const wallet = Object.fromEntries(
    (data?.collections.wallet ?? []).map((r) => [r.entity_key, r.values.count]),
  );
  return (
    <CatalogIconVersion.Provider value={catalogVersion.current}>
      <div className="layout">
        <aside className="sidebar">
          <a className="brand" href="/" aria-label={tr("Vision Overlay, главная")}>
            <span className="brand-mark">◇</span>
            <span>
              VISION<span className="brand-sub">OVERLAY</span>
            </span>
          </a>
          <nav>
            {[
              ["profile", "◈", tr("Профиль")],
              ["planner", "◎", tr("Планировщик")],
              ["dungeons", "▣", tr("Подземелья")],
              ["guides", "▤", tr("Гайды")],
              ["map", "⌖", tr("Интерактивная карта")],
            ].map(([id, icon, title]) => (
              <button
                key={id}
                className={tab === id ? "nav active" : "nav"}
                onClick={() => {
                  if (id === "guides" || id === "map") {
                    window.location.hash = id;
                    return;
                  }
                  setTab(id);
                  setSearch("");
                }}
              >
                <span>{icon}</span>
                {title}
              </button>
            ))}
            <details className="settings-nav" open={settingsOpen} onToggle={e=>setSettingsOpen(e.currentTarget.open)}>
              <summary className={`nav settings-toggle${["history","captures","backups","maintenance","catalog"].includes(tab)?" active":""}`}><span>⚙</span>{tr("Настройки")}</summary>
              <div className="settings-subnav">{[["history","↔",tr("История изменений")],["captures","◷",tr("Захваты и снимки")],["backups","⇩",tr("Резервные копии")],["maintenance","◴",tr("Хранение данных")],["catalog","⌘",tr("Справочник")]].map(([id,icon,title])=><button key={id} className={tab===id?"nav active":"nav"} onClick={()=>{setTab(id);setSearch("");}}><span>{icon}</span>{title}</button>)}</div>
            </details>
          </nav>
          <select className="language-picker" aria-label={language === "en" ? "Interface language" : "Язык интерфейса"}
            value={language} onChange={event => {
              sessionStorage.setItem("vo.language.return-tab", baseTab);
              setLanguage(event.target.value === "en" ? "en" : "ru");
            }}>
            <option value="ru" lang="ru">Русский</option>
            <option value="en" lang="en">English</option>
          </select>
          <div className="sidebar-bottom">
            <span className={"dot " + (app.connected ? "green" : "amber")} />
            {app.connected ? tr("Связь с приложением") : tr("Восстанавливаем связь")}
            <small>{tr("Локальное хранилище · SQLite")}</small>
          </div>
        </aside>
        <main>
          <header className="topbar">
            <div className="profile-picker">
                <select
                  aria-label={tr("Профиль")}
                  value={app.selected}
                  title={tr("Просмотр сохранённых профилей. При новом входе профиль переключится автоматически.")}
                  onChange={e=>app.setSelected(e.target.value)}
                >
                  {app.state?.profiles.map((p) => (
                    <option key={p.profile_id} value={p.profile_id} title={regionNames[p.server_region]}>
                      {headerRegionCodes[p.server_region] ?? p.server_region} ·{" "}
                      {p.uid}
                    </option>
                  ))}
                </select>
                <span className={"pill " + (current ? "live" : "")}>
                  {current ? tr("В игре") : tr("Сохранённый профиль")}
                </span>
                {!current&&app.state?.active_game_profile_id&&<button className="follow-game" onClick={app.followGame}>{tr("К аккаунту в игре")}</button>}
            </div>
            <div className="header-stats" aria-label={tr("Ранг и ресурсы выбранного профиля")}>
              <Stat label="AR" fullLabel={tr("Ранг приключений")} value={fmt(data?.collections.adventure?.[0]?.values.rank)} symbol="⌁" />
              <span className="header-divider" aria-hidden="true" />
              <Stat label={tr("Мора")} value={fmt(wallet["202"])} symbol="◈" />
              <Stat label={tr("Камни Истока")} value={fmt(wallet["201"])} symbol="✦" />
              <span className="header-divider" aria-hidden="true" />
              <ResinStat count={resin?.values.count} nextRecovery={resin?.values.next_recovery_unix_s}
                live={resin?.confirmation === "observed" && (data?.domains.some(d => d.collection === "wallet" && d.freshness === "synchronized") ?? false)} />
              <span className="header-divider" aria-hidden="true" />
              <div className="header-cycles">
                <AbyssStat values={data?.collections.abyss?.[0]?.values as React.ComponentProps<typeof AbyssStat>["values"]} />
                <TheaterStat values={data?.collections.theater?.[0]?.values as React.ComponentProps<typeof TheaterStat>["values"]} />
                <OnslaughtStat values={data?.collections.onslaught?.[0]?.values as React.ComponentProps<typeof OnslaughtStat>["values"]} />
              </div>
            </div>
            <button
              className="icon-button"
              onClick={app.reload}
              title={tr("Обновить данные")}
              aria-label={tr("Обновить данные")}
            >
              ↻
            </button>
          </header>
          <div className="content">
            <div className="heading">
              <h1>
                {
                  {
                    profile: tr("Профиль игрока"),
                    teams: tr("Сохранённые отряды"),
                    inventory: tr("Инвентарь"),
                    characters: tr("Персонажи"),
                    achievements: tr("Достижения"),
                    wishes: tr("Молитвы"),
                    planner: tr("Планировщик"),
                    dungeons: tr("Подземелья"),
                    guides: tr("Гайды"),
                    map: tr("Интерактивная карта"),
                    history: tr("История изменений"),
                    captures: tr("Захваты и снимки"),
                    backups: tr("Резервные копии"),
                    maintenance: tr("Хранение данных"),
                    catalog: tr("Справочник"),
                    abyss: tr("Витая Бездна"),
                    theater: tr("Театр Воображариум"),
                    onslaught: tr("Натиск"),
                  }[tab]
                }
              </h1>
              {tab === "inventory" && <p>{tr("Собранные ресурсы и экипировка вашего аккаунта.")}</p>}
              {tab === "characters" && <p>{tr("Характеристики, таланты и экипировка персонажей.")}</p>}
            </div>
            {(app.error || app.dataError) && (
              <div className="banner error" role="alert">{tr("Не удалось обновить данные. ")}{app.dataError || app.error}{" "}
                <button onClick={app.reload}>{tr("Повторить")}</button>
              </div>
            )}
            {!app.connected && data && (
              <div className="banner">{tr("Связь с приложением восстанавливается. Показаны последние загруженные данные; их актуальность пока не подтверждена.")}</div>
            )}
            {catalogError && (
              <div className="banner">{tr("Справочник названий недоступен. Предметы показаны по ID.")}</div>
            )}
            {tab === "abyss" ? (
              <AbyssView profileRegion={profile?.server_region}
                progress={data?.collections.abyss?.[0]?.values as React.ComponentProps<typeof AbyssView>["progress"]} />
            ) : tab === "theater" ? (
              <TheaterView profileRegion={profile?.server_region} catalog={catalog}
                progress={data?.collections.theater?.[0]?.values as React.ComponentProps<typeof TheaterView>["progress"]} />
            ) : tab === "onslaught" ? (
              <OnslaughtView profileRegion={profile?.server_region} catalog={catalog}
                progress={data?.collections.onslaught?.[0]?.values as React.ComponentProps<typeof OnslaughtView>["progress"]}
                exalted={data?.collections.onslaught_exalted?.[0]?.values as React.ComponentProps<typeof OnslaughtView>["exalted"]}
                exaltedObservedMs={data?.domains.find(domain => domain.collection === "onslaught_exalted")?.observed_unix_ms} />
            ) : tab === "dungeons" ? (
              <DungeonsView key={profile?.profile_id} profile={profile} data={data} catalog={catalog}
                openPlanner={() => { setTab("planner"); window.scrollTo(0, 0); }} />
            ) : tab === "guides" ? (
              <GuidesView catalog={catalog} selectedId={linkedTab?.split("/")[1]} catalogError={catalogError} />
            ) : tab === "map" ? (
              <WorldMapView key={profile?.profile_id} profile={profile} data={data} catalog={catalog}
                openPlanner={() => { setTab("planner"); window.scrollTo(0, 0); }} />
            ) : !app.state ? (
              <div className="empty">{tr("Подключаемся к локальному приложению…")}</div>
            ) : tab === "catalog" ? null : !profile ? (
              <div className="empty">
                <h2>{tr("Профилей пока нет")}</h2>
                <p>{tr("Запустите захват и войдите в игру, чтобы сохранить первый снимок.")}</p>
              </div>
            ) : !data ? (
              <div className="empty">{tr("Загружаем профиль…")}</div>
            ) : (
              <>
                {(tab === "inventory" || tab === "characters") && (
                  <>
                    <section className="panel">
                      {tab === "inventory" && (
                        <div
                          className="categories"
                          role="group"
                          aria-label={tr("Категории инвентаря")}
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
                              {tr(label)}
                              <span>{inventory[c]?.length ?? 0}</span>
                            </button>
                          ))}
                        </div>
                      )}
                      <div className="toolbar">
                        <label className="search">
                          <span>⌕</span>
                          <input
                            aria-label={tr("Поиск")}
                            placeholder={
                              tab === "characters"
                                ? tr("Имя персонажа или ID…")
                                : tr("Название, ID или GUID…")
                            }
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                          />
                          {search && (
                            <button
                              aria-label={tr("Очистить поиск")}
                              onClick={() => setSearch("")}
                            >
                              ×
                            </button>
                          )}
                        </label>
                        <select
                          aria-label={tr("Сортировка")}
                          value={tab === "characters" && sort === "count" ? "name" : sort}
                          onChange={(e) => setSort(e.target.value)}
                        >
                          <option value="name">{tr("По названию")}</option>
                          <option value="game">
                            {tab === "inventory"
                              ? tr("Порядок по справочнику игры")
                              : tr("По имени персонажа")}
                          </option>
                          {tab === "inventory" && <option value="count">{tr("По количеству")}</option>}
                          <option value="level">{tr("По уровню")}</option>
                        </select>
                        {tab === "inventory" && (
                          <>
                            <select
                              aria-label={tr("Тип предмета")}
                              value={subtype}
                              onChange={(e) =>
                                setInventoryFilter("subtype", e.target.value)
                              }
                            >
                              <option value="">{tr("Все типы")}</option>
                              {subtype &&
                                !subtypeOptions(
                                  rawRows,
                                  catalog.items ?? {},
                                ).some(([type]) => type === subtype) && (
                                  <option value={subtype}>{gameText(tr(subtype))} · 0</option>
                                )}
                              {subtypeOptions(rawRows, catalog.items ?? {}).map(
                                ([type, count]) => (
                                  <option key={type} value={type}>
                                    {gameText(tr(type))} · {count}
                                  </option>
                                ),
                              )}
                            </select>
                            <select
                              aria-label={tr("Редкость")}
                              value={rarityFilter}
                              onChange={(e) =>
                                setInventoryFilter("rarity", e.target.value)
                              }
                            >
                              <option value="">{tr("Любая редкость")}</option>
                              {[5, 4, 3, 2, 1, 0].map((r) => (
                                <option key={r} value={String(r)}>
                                  {r ? `${r} ★` : tr("Без редкости")}
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
                              >{tr("Сбросить фильтры")}</button>
                            )}
                            <div
                              className="view-toggle"
                              role="group"
                              aria-label={tr("Вид инвентаря")}
                            >
                              <button
                                aria-pressed={inventoryView === "grid"}
                                className={
                                  inventoryView === "grid" ? "selected" : ""
                                }
                                onClick={() => setInventoryView("grid")}
                              >{tr("▦ Сетка")}</button>
                              <button
                                aria-pressed={inventoryView === "list"}
                                className={
                                  inventoryView === "list" ? "selected" : ""
                                }
                                onClick={() => setInventoryView("list")}
                              >{tr("☷ Список")}</button>
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
                            />{tr("Режим разработчика")}</label>
                        )}
                        <span className="muted">{tab === "characters" ? tr("Получено: {0}", gallery.filter((r:any)=>!r.unowned).length) : tr("Найдено: {0}", rows.length)}</span>
                      </div>
                      {tab === "characters" && <div className="character-element-filters" role="group" aria-label={tr("Фильтр по стихиям")}>
                        <button aria-pressed={!selectedElements.length} onClick={()=>setSelectedElements([])}>{tr("Все стихии")}</button>
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
                          : tr("Нет снимка для этого раздела")}
                        <span className="muted">
                          {domain
                            ? tr("Обновлено {0}", date(domain.observed_unix_ms))
                            : ""}
                        </span>
                        {domain?.freshness === "stale" && (
                          <b>{tr("Нужен свежий снимок")}</b>
                        )}
                      </div>
                      {domain?.scope !== "full" && (
                        <div className="coverage">{tr("Показаны наблюдавшиеся записи. Отсутствие предмета в неполном снимке не подтверждает нулевое количество.")}</div>
                      )}
                      <div ref={virtual.ref} className="virtual-items">
                      <div style={{height:virtual.top}} aria-hidden="true" />
                      {rows.length === 0 ? (
                        <div className="empty">
                          <h2>
                            {search || subtype || rarityFilter
                              ? tr("Ничего не найдено")
                              : tr("В этом разделе пока пусто")}
                          </h2>
                          <p>
                            {search || subtype || rarityFilter
                              ? tr("Измените поиск или сбросьте фильтры.")
                              : tr("Данные появятся после подходящего снимка.")}
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
                                <th>{tr("Предмет")}</th>
                                <th>
                                  {!["weapons", "artifacts"].includes(category)
                                    ? tr("Количество")
                                    : category === "weapons"
                                      ? tr("Уровень / количество")
                                      : tr("Уровень")}
                                </th>
                                <th>
                                  {category === "weapons"
                                    ? tr("Пробуждение / редкость")
                                    : tr("Редкость")}
                                </th>
                                <th>{tr("Статус")}</th>
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
                                        ? tr("Ур. {0}", fmt(r.values.level))
                                        : tr("{0} шт.", fmt(r.values.count))
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
                                        ? tr("Подтверждён")
                                        : tr("Из прошлых данных")}
                                      {r.values.locked ? " · 🔒" : ""}
                                    </span>
                                  </td>
                                  <td>
                                    <button
                                      className="text-button"
                                      onClick={() => setDetail(r)}
                                    >{tr("Подробнее ↗")}</button>
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
                      <p className="footnote">{tr("Игровые изображения © HoYoverse. Источники:")}{" "}
                        <a
                          href="https://enka.network/"
                          target="_blank"
                          rel="noreferrer"
                        >
                          Enka.Network
                        </a>{" "}{tr("и")}{" "}
                        <a
                          href="https://github.com/PathOfGenshin/resources"
                          target="_blank"
                          rel="noreferrer"
                        >
                          PathOfGenshin
                        </a>{tr(". Загруженные иконки сохраняются на этом компьютере.")}</p>
                    )}
                    {tab === "characters" && (
                      <p className="footnote">{tr("Итоговые характеристики отражают последние полученные от сервера значения.")}</p>
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
                    navigate={section=>{
                      setTab(section);
                      setSearch("");
                      window.scrollTo(0, 0);
                    }}
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
                {tab === "planner" && <Planner key={app.selected} profile={app.selected} tick={app.tick} data={data} catalog={catalog}
                  requestedGoal={goalRequest && goalRequest.profile===app.selected ? goalRequest.choice : undefined}
                  onRequestHandled={()=>setGoalRequest(null)}
                  openEntity={setDetail}
                  openMaterial={id=>{
                    const key=String(id);
                    const walletRow=data.collections.wallet?.find(row=>row.entity_key===key);
                    const materialRow=aggregate(data.collections.material_stacks??[]).find((row:Row)=>row.entity_key===key);
                    setDetail(walletRow ?? materialRow ?? {
                      entity_key:key, guids:[], confirmation:"historical_unconfirmed", value_format:"catalog", revision:data.revision,
                      values:{item_id:id,count:observedStock(data,id,id===202)},
                    });
                  }} />}
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
                    <b>{tr("Захват игровой сессии")}</b>
                    <p>{tr("Запускайте до входа в игру из PowerShell администратора. Остановка — Ctrl+C или выход из игры.")}</p>
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
                  >{tr("Скопировать команду")}</button>
                </section>
                {copied && (
                  <p className="footnote" role="status">{tr("Команда скопирована. Выполните её из папки Vision Overlay.")}</p>
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
              <span>{tr("Локальное приложение · Данные хранятся на этом компьютере")}</span>
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
            addGoal={choice=>{
              setGoalRequest({profile:app.selected,choice});
              setDetail(null);
              setDetailParent(null);
              setTab("planner");
            }}
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
  fullLabel,
  value,
  symbol,
}: {
  label: string;
  fullLabel?: string;
  value: string;
  symbol: string;
}) {
  return (
    <div className="header-stat">
      <span className="header-stat-symbol" aria-hidden="true">{symbol}</span>
      <div>
        <small>{fullLabel ? <abbr title={fullLabel}>{label}</abbr> : label}</small>
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
  addGoal,
}: {
  row: Row;
  data: ProfileData;
  catalog: Catalog;
  name: (v: any) => string;
  close: () => void;
  openItem: (item:Row)=>void;
  back?: ()=>void;
  addGoal: (choice:GoalChoice)=>void;
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
  const weapon = (live.source_collection ?? row.source_collection) === "weapons" ||
    (data.collections.weapons ?? []).some(item => item.entity_key === row.entity_key);
  const canPlan = isCharacter ? !referenceOnly && !v.trial_avatar_info && v.avatar_id !== 10000134 : weapon;
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
          aria-label={tr("Закрыть карточку")}
        >
          ×
        </button>
        {back&&<button className="detail-back" onClick={back}>{tr("‹ К персонажу")}</button>}
        <div className="eyebrow">{isCharacter ? tr("ПЕРСОНАЖ") : tr("ПРЕДМЕТ")}</div>
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
        <p className="muted">{isCharacter ? referenceOnly ? tr("Не получен") : tr("Дружба {0}", v.friendship?.level ?? "—") : `ID ${v.item_id}`}</p>
        {owner && <div className="detail-equipment-owner"><ItemIcon icon={catalog.avatarIcons?.[owner.values.avatar_id]} fallback={name(owner.values).slice(0,1)} /><div><small>{tr("Экипировано")}</small><strong>{name(owner.values)}</strong></div></div>}
        {isCharacter &&
          catalog.avatarMetadata?.[String(v.avatar_id)]?.weaponType ===
            "WEAPON_CROSSBOW" && (
            <p className="footnote">{tr("Отдельная запись персонажа игрового режима с винтовками. Она сохраняется в профиле отдельно от обычного Путешественника.")}</p>
          )}
        {!referenceOnly && <div className="detail-summary">
          {isCharacter
            ? tr("Уровень {0}", characterLevel(v))
            : v.kind === "weapon" ? tr("Уровень {0} · Возвышение {1}", v.level ?? "—", v.ascension ?? "—")
            : tr("Количество {0}{1}{2}", fmt(v.count), v.level !== undefined ? tr(" · Уровень ") + v.level : "", v.refinement !== undefined ? " · R" + v.refinement : "")}
        </div>}
        {canPlan && <button className="detail-plan-button" type="button" onClick={()=>addGoal({...live,collection:isCharacter?"characters":"weapons"})}>{tr("В планировщик")}</button>}
        {isTraveler(v) && (
          <p className="muted">{tr("Активная стихия: ")}{elementName(v.skill_depot_id, catalog)}
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
            <h3>{tr("Основная характеристика")}</h3>
            <p>
              {artifactInfo.main
                ? `${statName(artifactInfo.main.type)} · ${artifactInfo.main.value !== null ? statValue(artifactInfo.main.type, artifactInfo.main.value) : tr("значение не определено")}`
                : tr("Не определена (ID {0})", v.main_prop_id)}
            </p>
            <h3>{tr("Дополнительные характеристики")}</h3>
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
          <summary>{tr("Диагностические данные")}</summary>
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
          currency_changed: tr("Изменение валюты"),
          character_progress_changed: tr("Повышение уровня персонажа"),
          talent_changed: tr("Улучшение таланта"),
          equipment_changed: tr("Смена экипировки"),
          item_changed: tr("Изменение экипировки"),
          friendship_changed: tr("Изменение дружбы"),
          achievement_changed: tr("Изменение достижения"),
          constellation_changed: tr("Активация созвездия"),
          traveler_element_changed: tr("Обновление стихии Путешественника"),
          "state.invalidated": tr("Разрыв данных"),
          "capture.closed": tr("Захват завершён"),
          "identity.confirmed": tr("Профиль подтверждён"),
          "state.snapshot": tr("Получен снимок"),
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
          aria-label={tr("Поиск по истории")}
          placeholder={tr("Поиск по событиям…")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <span className="muted">
          {busy ? tr("Обновление…") : tr("{0} событий", visible.length)}
        </span>
      </div>
      {error && (
        <div role="alert" className="banner error">
          {error}
        </div>
      )}
      <div className="coverage">{tr("Показаны события, сохранённые приложением. Время указано по часовому поясу компьютера.")}{cursor
          ? tr(" Показаны последние события; более ранние записи можно загрузить ниже.")
          : ""}
      </div>
      {visible.length === 0 ? (
        <div className="empty">{tr("Подходящих событий пока нет")}</div>
      ) : (
        <div className="timeline">
          {visible.map((e) => (
            <details key={e.revision}>
              <summary>
                <time>{date(e.observed_unix_ms)}</time>
                <span>
                  {e.projection_result === "rejected" ? tr("⚠ Отклонено · ") : ""}
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
        >{tr("Загрузить следующие записи")}</button>
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
      ? tr("Нет данных")
      : row.values.status_code === 3
        ? tr("Награда получена")
        : row.values.status_code === 2
          ? tr("Награда доступна")
          : `${row.values.current_progress ?? "?"} / ${row.values.total_progress ?? "?"}`;
  return (
    <section className="panel achievements-panel">
      <div className="toolbar">
        <input
          aria-label={tr("Поиск достижений")}
          placeholder={tr("Название, описание или ID…")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <select
          aria-label={tr("Статус достижения")}
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        >
          <option value="all">{tr("Все статусы")}</option>
          <option value="unearned">{tr("Не получены")}</option>
          <option value="claimed">{tr("Награда получена")}</option>
          <option value="finished">{tr("Выполнены, награда не получена")}</option>
        </select>
        <select
          aria-label={tr("Категория достижений")}
          value={category}
          onChange={(e) => setCategory(e.target.value)}
        >
          <option value="all">{tr("Все категории")}</option>
          {categories.map(([id, meta]) => (
            <option key={id} value={id}>
              {meta.category ?? tr("Без категории")}
            </option>
          ))}
        </select>
      </div>
      <div className="domain-line">{tr("Найдено: ")}{rows.length}{tr(" · Награда получена: ")}{claimed}{" "}
        <span className="subtle">{tr("Счётчик наград учитывает каждый этап отдельно")}</span>
      </div>
      <div className="achievement-list">
        {rows.slice(0, limit).map((group, index) => (
          <React.Fragment key={group.key}>
            {(index === 0 ||
              rows[index - 1].meta.goal_id !== group.meta.goal_id) && (
              <h2 className="achievement-category">
                {group.meta.category ?? tr("Без категории")}
              </h2>
            )}
            <article className="achievement-card">
              <span className="achievement-symbol">✧</span>
              <div className="achievement-body">
                <div className="achievement-title">
                  <h3>{group.meta.name ?? tr("Достижение {0}", group.key)}</h3>
                  {group.levels.length > 1 && (
                    <div
                      className="achievement-stars"
                      aria-label={tr("Этапы достижения")}
                    >
                      {group.levels.map((level: any) => (
                        <span
                          key={level.id}
                          title={tr("Этап {0}: {1}", level.meta.stage ?? 1, stageStatus(level.row))}
                          aria-label={tr("Этап {0}: {1}", level.meta.stage ?? 1, stageStatus(level.row))}
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
                          <small className="stage-number">{tr("Этап ")}{level.meta.stage ?? 1}
                          </small>
                        )}
                        <p>{level.meta.description ?? tr("Описание недоступно")}</p>
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
        <div className="empty">{tr("Достижений по этим условиям нет")}</div>
      )}
      {rows.length > limit && (
        <button className="load-more" onClick={() => setLimit((n) => n + 80)}>{tr("Показать ещё")}</button>
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
          aria-label={tr("Захват")}
          value={capture}
          onChange={(e) => setCapture(e.target.value)}
        >
          <option value="">{tr("Выберите захват…")}</option>
          {captures.map((c) => (
            <option value={c.capture_id} key={c.capture_id}>
              {new Date(c.imported_at).toLocaleString(uiLocale)} ·{" "}
              {c.terminal_status} · {c.event_count}{tr(" событий")}</option>
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
          {selected.terminal_status} · {selected.event_count}{tr(" событий ·")}{" "}
          {selected.capture_id}
        </div>
      )}
      <DiagnosticExport capture={capture}/>
      {capture && snapshots.length === 0 && (
        <div className="empty">{tr("Снимков выбранного профиля в этом захвате нет")}</div>
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
            <small>{tr("Событие ")}{s.source_sequence} ·{" "}
              {s.scope === "full" ? tr("Полный снимок") : tr("Частичное покрытие")}
            </small>
          </button>
        ))}
      </div>
      {snapshot && (
        <div className="comparison">
          <h3>
            {labels[snapshot.collection] ?? snapshot.collection}{tr(": сравнение с текущим состоянием")}</h3>
          <p>
            {Object.keys(snapshot.entities).length}{tr(" записей в снимке ·")}{" "}
            {diff.length}{tr(" различий")}</p>
          <div className="coverage">{tr("Отсутствие записи в частичном снимке означает «не наблюдалась». Сравнение не меняет выбранный профиль или его данные.")}</div>
          {diff.slice(0, 100).map((d: any) => (
            <div className="difference" key={d.key}>
              <span>
                {name(d.after ?? d.before)}
                <small>{tr("GUID / ключ ")}{d.key}</small>
              </span>
              <b>
                {d.before?.count !== undefined || d.after?.count !== undefined
                  ? `${d.before?.count ?? tr("не наблюдался")} → ${d.after?.count ?? tr("не наблюдался")}`
                  : d.kind === "added"
                    ? tr("Новая запись")
                    : d.kind === "removed"
                      ? tr("Нет в текущей базе")
                      : tr("Изменены характеристики")}
              </b>
            </div>
          ))}
          {diff.length > 100 && <p>{tr("Показаны первые 100 различий.")}</p>}
          <details>
            <summary>{tr("Полные данные исходного снимка")}</summary>
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
    helper_unavailable: tr("Наблюдатель не отвечает. Проверьте его запуск."),
    helper_upgrade_required: tr("Наблюдатель нужно обновить и перезапустить."),
    already_running: tr("Захват уже запущен."),
    run_changed: tr("Захват уже завершился или сменился. Обновите статус."),
    command_pending: tr("Предыдущая команда ещё обрабатывается."),
    helper_restarted: tr("Наблюдатель перезапустился. Повторите команду."),
    expired_command: tr("Команда не была обработана вовремя."),
    launch_failed: tr("Не удалось запустить захват. Проверьте журнал наблюдателя."),
    capture_failed: tr("Захват завершился с ошибкой. Проверьте журнал."),
    invalid_command: tr("Команда отклонена наблюдателем."),
    observation_failed: tr("Не удалось проверить процессы игры."),
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
              tr("Наблюдатель перезапустился. Проверьте состояние захвата перед повторной командой."),
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
                  ? tr("Источник готов. Для синхронизации дождитесь снимка после входа в игру.")
                  : tr("Захват завершён. Запись финализирована; бэкенд завершает доставку оставшихся событий.")
                : (errors[result.code] ??
                    tr("Команда не выполнена: ") + result.code),
            );
            setPending(null);
            pendingRef.current = null;
          } else if (Date.now() - p.at > 45000) {
            setFeedback(
              tr("Подтверждение команды не получено. Проверьте состояние и журнал наблюдателя."),
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
            tr("Команда не выполнена: ") + (result.error?.code ?? response.status),
        );
        setPending(null);
        pendingRef.current = null;
      }
    } catch {
      setFeedback(tr("Ответ сервера не получен; уточняем состояние команды…"));
    }
  }
  const titles: Record<string, string> = {
    not_configured: tr("Не настроен"),
    disabled: tr("Отключён"),
    idle: tr("Ручной режим · захват не запущен"),
    watcher_unavailable: tr("Наблюдатель не отвечает"),
    waiting: tr("Захват не запущен"),
    capturing: tr("Захват работает"),
    capture_started: tr("Запуск захвата"),
    capture_finished: tr("Захват завершён"),
    capture_failed: tr("Ошибка захвата"),
    launch_failed: tr("Ошибка запуска"),
    already_running: tr("Ожидание нового входа в игру"),
    external_capture: tr("Работает внешний захват"),
    handled: tr("Захват не запущен"),
    observation_failed: tr("Не удалось проверить процесс игры"),
    starting: tr("Запуск источника…"),
    ready: tr("Захват работает · источник готов"),
    stopping: tr("Завершение записи…"),
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
          <b>{tr("Управление захватом")}</b>
          <p>
            <span
              className={
                "dot " + (status?.control_available ? "green" : "amber")
              }
            />
            {error
              ? tr("Статус временно недоступен")
              : (titles[phase] ?? tr("Проверка состояния…"))}
          </p>
        </div>
        <div className="capture-buttons">
          <button
            disabled={
              !available || !!status.capture_run_id || status.external_capture
            }
            onClick={() => void control("start")}
          >
            {pending?.action === "start" ? tr("Запускаем…") : tr("Начать захват")}
          </button>
          <button
            disabled={
              !available || !status.capture_run_id || phase === "stopping"
            }
            onClick={() => void control("stop")}
          >
            {pending?.action === "stop"
              ? tr("Останавливаем…")
              : tr("Остановить захват")}
          </button>
        </div>
      </div>
      <p>{tr("Запускайте захват до входа в игру и дождитесь готовности источника. При запуске посреди игры для получения снимка потребуется повторный вход в игровую сессию. Ручная остановка сохраняет запись и не запускает захват заново автоматически в том же процессе игры.")}</p>
      {status?.watcher_alive && !status?.control_available && (
        <p className="control-message">{tr("Для кнопок управления нужно обновить работающий наблюдатель.")}</p>
      )}
      {status?.external_capture && (
        <p className="control-message">{tr("Этот захват запущен вне помощника. Остановите его через Ctrl+C в исходном терминале.")}</p>
      )}
      {feedback && (
        <p className="control-message" role="status">
          {feedback}
        </p>
      )}
      <div className="automation-heading auto-settings">
        <div>
          <b>{tr("Автоматический запуск")}</b>
          <p>
            {status?.enabled ? tr("Включён") : tr("Отключён")} ·{" "}
            {status?.watcher_alive
              ? tr("наблюдатель работает")
              : tr("наблюдатель недоступен")}
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
          {status?.enabled ? tr("Команда отключения") : tr("Команда включения")}
        </button>
      </div>
      <details>
        <summary>{tr("Настройка и ограничения")}</summary>
        <p>
          {status?.portable
            ? tr("Настройте захват через меню значка Vision Overlay в трее. Для ручной настройки можно выполнить команду ниже в PowerShell администратора.")
            : tr("Однократно выполните команду ниже в PowerShell администратора из папки Vision Overlay.")}
          {" "}{tr("Помощник работает от вашего пользователя с повышенными правами. Пароль не сохраняется.")}</p>
        <pre>{command}</pre>
        <p>{tr("Помощник принимает только запуск и штатную остановку захвата. Отключение автоматического старта оставляет ручные кнопки доступными. При выходе из игры захват завершится самостоятельно. Журналы:")}{status?.portable ? "data/auto-capture." : "local-data/auto-capture."}
        </p>
      </details>
      {copied && (
        <p role="status">{tr("Команда скопирована. Выполните её в PowerShell администратора.")}</p>
      )}
    </section>
  );
}
document.documentElement.lang = language;
void loadGameLanguage().catch(console.error).finally(() => {
  createRoot(document.getElementById("root")!).render(<App />);
});
