import { tr } from "./i18n.mjs";
import { ItemIcon } from "./InventoryViews";
import { elements } from "./CharacterGrid";
import { regions } from "./model.mjs";
import { useEndgameCycle, type CycleProgress, type CycleSchedule } from "./useEndgameCycle";

type TheaterSeason = CycleSchedule & {
  elements: string[];
  opening: number[];
  guests: number[];
  stages: {
    title: string;
    variants: { label: string; enemies: string[] }[];
  }[];
};

type TheaterProgress = CycleProgress & { acts: number; arcana?: number | null; echo_keys?: number | null };
type AvatarCatalog = { avatarNames: Record<string, string>; avatarIcons?: Record<string, string> };

export function TheaterView({ profileRegion, progress, catalog }: {
  profileRegion?: string;
  progress?: TheaterProgress;
  catalog: AvatarCatalog;
}) {
  const { current, result, hasProfileRegion, region, loading, ends, endsLabel, setRegion } =
    useEndgameCycle<TheaterSeason, TheaterProgress>("theater", profileRegion, progress);

  return <section className="panel abyss-panel" aria-label={tr("Текущий сезон Театра Воображариум")}>
    <div className="abyss-overview">
      <div>
        <strong>{hasProfileRegion ? regions[region] : tr("Регион")}</strong>
        {!hasProfileRegion && <select aria-label={tr("Регион Театра")} value={region}
          onChange={event => setRegion(event.target.value)}>
          {Object.entries(regions).map(([id, name]) => <option key={id} value={id}>{name}</option>)}
        </select>}
        {current && <div className="muted">{tr("Обновится ")}<time dateTime={new Date(ends).toISOString()}>{endsLabel}</time>
        </div>}
      </div>
      {result && <div className="abyss-progress">
        <strong>{tr("Акты ")}{result.acts}/10 · {result.arcana === 2 ? tr("Арканы 2/2") :
          result.arcana === 1 ? tr("Арканы ≥1/2") : tr("Данные об арканах неизвестны")}</strong>
        {result.echo_keys != null && <span>{tr("Ключи эха за сезон ")}{result.echo_keys}/2</span>}
      </div>}
    </div>
    {current ? <div className="abyss-floor theater-season">
      {result?.arcana !== 2 && <p className="banner theater-refresh-hint">{tr("Для актуализации данных откройте страницу результатов текущего Театра.")}</p>}
      <section className="abyss-buffs">
        <h2>{tr("Стихии")}</h2>
        <ul className="theater-elements">{current.elements.map(element => {
          const [label, fallback] = elements[element] ?? [element, "?"];
          return <li key={element}>
            <ItemIcon icon={elements[element] ? `UI_Buff_Element_${element}` : undefined} fallback={fallback} />
            <span>{label}</span>
          </li>;
        })}</ul>
      </section>
      <div className="theater-cast">
        {([
          [tr("Открывающие персонажи"), current.opening],
          [tr("Специально приглашённые персонажи"), current.guests],
        ] as const).map(([label, ids]) => <section className="abyss-chamber" key={label}>
          <h2>{label}</h2>
          <ul className="theater-avatars">
            {ids.map(id => {
              const name = catalog.avatarNames[id] ?? tr("Персонаж {0}", id);
              return <li key={id}>
                <ItemIcon icon={catalog.avatarIcons?.[id]} fallback={name.slice(0, 1)} large />
                <span>{name}</span>
              </li>;
            })}
          </ul>
        </section>)}
      </div>
      <section>
        <h2>{tr("Противники по актам")}</h2>
        <p className="muted">{tr("В актах с выбором события показаны возможные группы противников.")}</p>
        <div className="abyss-chambers">
          {current.stages.map(stage => <article className="abyss-chamber" key={stage.title}>
            <h2>{stage.title}</h2>
            {stage.variants.map(variant => <section key={variant.label}>
              <h3>{variant.label}</h3>
              <ul>{variant.enemies.map(enemy => <li key={enemy}>{enemy}</li>)}</ul>
            </section>)}
          </article>)}
        </div>
      </section>
    </div> : <div className="empty" role="status">
      {loading ? tr("Загружаем данные текущего сезона…") : tr("Данные текущего сезона пока недоступны.")}
    </div>}
  </section>;
}
