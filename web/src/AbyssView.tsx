import { tr } from "./i18n.mjs";
import { useState } from "react";
import { regions } from "./model.mjs";
import { useEndgameCycle, type CycleProgress, type CycleSchedule } from "./useEndgameCycle";

type AbyssCycle = CycleSchedule & {
  floors: {
    floor: number;
    buffs: { half: number | null; text: string }[];
    chambers: {
      chamber: number;
      level: number;
      halves: { half: number; enemies: { describe_id: number; name: string }[] }[];
    }[];
  }[];
};

type AbyssProgress = CycleProgress & {
  floor?: number;
  chamber?: number;
  stars: number;
};

const halfName = (half: number) => half === 1 ? tr("Первая половина") : tr("Вторая половина");

export function AbyssView({ profileRegion, progress }: {
  profileRegion?: string;
  progress?: AbyssProgress;
}) {
  const [selectedFloor, setSelectedFloor] = useState(12);
  const { current, result, hasProfileRegion, region, loading, ends, endsLabel, setRegion } =
    useEndgameCycle<AbyssCycle, AbyssProgress>("abyss", profileRegion, progress);
  const floors = current?.floors ?? [];
  const floor = floors.find(entry => entry.floor === selectedFloor) ?? floors.at(-1);

  return <section className="panel abyss-panel" aria-label={tr("Текущий цикл Витой Бездны")}>
    <div className="abyss-overview">
      <div>
        <strong>{hasProfileRegion ? regions[region] : tr("Регион")}</strong>
        {!hasProfileRegion && <select aria-label={tr("Регион Бездны")} value={region}
          onChange={event => setRegion(event.target.value)}>
          {Object.entries(regions).map(([id, name]) => <option key={id} value={id}>{name}</option>)}
        </select>}
        {current && <div className="muted">{tr("Обновится ")}<time dateTime={new Date(ends).toISOString()}>{endsLabel}</time>
        </div>}
      </div>
      {result && <div className="abyss-progress">
        <strong>{result.stars}/36 ★</strong>
        {result.floor && result.chamber ? <span>{tr("Этаж ")}{result.floor}{tr(", зал ")}{result.chamber}</span> : null}
      </div>}
    </div>
    {current && floor ? <>
      <div className="categories" role="group" aria-label={tr("Этажи")}>
        {floors.map(entry => <button key={entry.floor} type="button"
          aria-pressed={entry.floor === floor.floor}
          className={entry.floor === floor.floor ? "selected" : ""}
          onClick={() => setSelectedFloor(entry.floor)}>{tr("Этаж ")}{entry.floor}</button>)}
      </div>
      <div className="abyss-floor">
        <div className="abyss-buffs">
          <h2>{tr("Аномалия артерий земли")}</h2>
          {floor.buffs.map(buff => <p key={buff.text}>
            {buff.half && <strong>{halfName(buff.half)} · </strong>}{buff.text}
          </p>)}
        </div>
        <div className="abyss-chambers">
          {floor.chambers.map(chamber => <article className="abyss-chamber" key={chamber.chamber}>
            <h2>{tr("Зал ")}{chamber.chamber} <small>{tr("Ур. ")}{chamber.level}</small></h2>
            {chamber.halves.map(half => <section key={half.half}>
              <h3>{halfName(half.half)}</h3>
              <ul>{half.enemies.map(enemy =>
                <li key={enemy.describe_id}>{enemy.name}</li>)}</ul>
            </section>)}
          </article>)}
        </div>
      </div>
    </> : <div className="empty" role="status">
      {loading ? tr("Загружаем данные текущего цикла…") : tr("Данные текущего цикла пока недоступны.")}
    </div>}
  </section>;
}
