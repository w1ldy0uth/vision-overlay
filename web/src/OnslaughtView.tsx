import { tr, uiLocale } from "./i18n.mjs";
import { useState } from "react";
import { onslaughtDifficulties } from "./AbyssStat";
import { ItemIcon } from "./InventoryViews";
import { regions } from "./model.mjs";
import { useEndgameCycle, type CycleProgress, type CycleSchedule } from "./useEndgameCycle";

type OnslaughtCycle = CycleSchedule & {
  name: string;
  bosses: {
    id: number;
    name: string;
    icon: string;
    forms: [string, string, string];
    advantages: string[];
    disadvantages?: string[];
  }[];
};

type OnslaughtProgress = CycleProgress & { difficulty: number; seconds: number };
type OnslaughtExalted = CycleProgress & { avatar_ids: number[] };
type AvatarCatalog = { avatarNames: Record<string, string>; avatarIcons?: Record<string, string> };
const dateTime = new Intl.DateTimeFormat(uiLocale, { dateStyle: "medium", timeStyle: "short" });

export function OnslaughtView({ profileRegion, progress, exalted, exaltedObservedMs, catalog }: {
  profileRegion?: string;
  progress?: OnslaughtProgress;
  exalted?: OnslaughtExalted;
  exaltedObservedMs?: string;
  catalog: AvatarCatalog;
}) {
  const { current, result, hasProfileRegion, region, loading, starts, ends, endsLabel, setRegion } =
    useEndgameCycle<OnslaughtCycle, OnslaughtProgress>("onslaught", profileRegion, progress);
  const [selected, setSelected] = useState<number>();
  const difficulty = selected ?? result?.difficulty ?? 4;
  const formIndex = difficulty <= 3 ? 0 : difficulty === 4 ? 1 : 2;
  const capturedAt = Number(exaltedObservedMs);
  const roster = hasProfileRegion && exalted && exalted.schedule_id === current?.schedule_id
    && exalted.starts_unix_s * 1000 === starts && (exalted.ends_unix_s + 1) * 1000 === ends
    && capturedAt >= starts && capturedAt < ends
    ? { ids: exalted.avatar_ids, observedAt: capturedAt }
    : null;

  return <section className="panel" aria-label={tr("Текущий цикл Натиска")}>
    <div className="abyss-overview">
      <div>
        <strong>{hasProfileRegion ? regions[region] : tr("Регион")}</strong>
        {!hasProfileRegion && <select aria-label={tr("Регион Натиска")} value={region}
          onChange={event => setRegion(event.target.value)}>
          {Object.entries(regions).map(([id, name]) => <option key={id} value={id}>{name}</option>)}
        </select>}
        {current && <div className="muted">
          {current.name}{tr(" · Завершится ")}<time dateTime={new Date(ends).toISOString()}>{endsLabel}</time>
        </div>}
      </div>
      {result && <div className="abyss-progress">
        <strong>{tr("Сложность ")}{onslaughtDifficulties[result.difficulty - 1]}</strong>
        <span>{tr("Лучший рекорд · ")}{result.seconds}{tr(" с")}</span>
      </div>}
    </div>
    {current?.bosses?.length ? <>
      <div className="categories" role="group" aria-label={tr("Сложность Натиска")}>
        {onslaughtDifficulties.map((label, index) => <button key={label} type="button"
          className={difficulty === index + 1 ? "selected" : ""}
          aria-pressed={difficulty === index + 1}
          onClick={() => setSelected(index + 1)}>{tr("Сложность ")}{label}</button>)}
      </div>
      <div className="abyss-floor onslaught-detail">
        <div className="abyss-chambers">
          {current.bosses.map(boss => <article className="abyss-chamber" key={boss.id}>
            <div className="onslaught-boss">
              <ItemIcon icon={boss.icon} fallback={boss.name.slice(0, 1)} large />
              <div><h2>{boss.name}</h2><span className="muted">{boss.forms[formIndex]}</span></div>
            </div>
            <h3>{tr("Преимущество")}</h3>
            <ul>{boss.advantages.map(item => <li key={item}>{item}</li>)}</ul>
            {!!boss.disadvantages?.length && <>
              <h3>{tr("Недостаток")}</h3>
              <ul>{boss.disadvantages.map(item => <li key={item}>{item}</li>)}</ul>
            </>}
          </article>)}
        </div>
        <section className="abyss-buffs onslaught-buffs">
          <h2>{tr("Вознесённые")}</h2>
          {difficulty === 6 ? <p>{tr("На сложности VI усиление персонажей не действует.")}</p> : <>
            <p>{tr("Во время испытания максимальное HP, сила атаки и защита Вознесённых увеличиваются на 20%.")}</p>
            {roster ? <>
              <p className="muted">{tr("Последний снимок · ")}{dateTime.format(roster.observedAt)}</p>
              <ul className="theater-avatars onslaught-avatars">
                {roster.ids.map(id => {
                  const name = catalog.avatarNames[id] ?? tr("Персонаж {0}", id);
                  return <li key={id}>
                    <ItemIcon icon={catalog.avatarIcons?.[id]} fallback={name.slice(0, 1)} large />
                    <span>{name}</span>
                  </li>;
                })}
              </ul>
            </> : <p className="muted">{tr("Для этого региона снимок списка пока недоступен. Откройте «Вознесённые» в меню Натиска.")}</p>}
          </>}
        </section>
      </div>
    </> : <div className="empty" role="status">
      {loading ? tr("Загружаем данные текущего цикла…") : tr("Данные текущего цикла пока недоступны.")}
    </div>}
  </section>;
}
