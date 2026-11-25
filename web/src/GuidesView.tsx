import { tr, uiLocale } from "./i18n.mjs";
import { useMemo, useState } from "react";
import { ItemIcon } from "./InventoryViews";
import { characterGuides, guideAvatarId, guideCharacters, guideSections, travelerGuideId, type GuideBuild, type GuideCatalog } from "./character-guides";

export function GuidesView({ catalog, selectedId, catalogError }: {
  catalog: GuideCatalog;
  selectedId?: string;
  catalogError: boolean;
}) {
  const [search, setSearch] = useState("");
  const characters = useMemo(() => guideCharacters(catalog), [catalog]);
  if (!catalog.avatarMetadata) {
    return <div className="empty">{catalogError ? tr("Справочник персонажей недоступен.") : tr("Загружаем список персонажей…")}</div>;
  }

  if (selectedId) {
    const character = characters.find(entry => entry.id === guideAvatarId(selectedId));
    return <div className="guide-detail">
      <a className="guide-back" href="#guides">{tr("← Все персонажи")}</a>
      {character ? <GuideCard character={character} /> : <div className="empty">{tr("Персонаж не найден.")}</div>}
    </div>;
  }

  const query = search.trim().toLocaleLowerCase(uiLocale);
  const visible = characters.filter(({ id, name }) => name.toLocaleLowerCase(uiLocale).includes(query) ||
    (id === travelerGuideId && tr("путешественница").includes(query)));
  return <section className="panel guide-list" aria-label={tr("Гайды по персонажам")}>
    <p className="guide-development">{tr("В разработке")}</p>
    <input type="search" aria-label={tr("Поиск персонажа")} placeholder={tr("Поиск по имени")} value={search}
      onChange={event => setSearch(event.target.value)} />
    {visible.length ? <ul className="guide-roster">
      {visible.map(({ id, name, icon }) => <li key={id}>
        <a href={`#guides/${id}`}>
          <ItemIcon icon={icon} fallback={name.slice(0, 1)} />
          <span>{name}</span>
        </a>
      </li>)}
    </ul> : <div className="empty">{tr("Персонажи не найдены.")}</div>}
  </section>;
}

function GuideCard({ character }: { character: ReturnType<typeof guideCharacters>[number] }) {
  const guide = characterGuides[character.id];
  const builds: GuideBuild[] = guide?.builds?.length ? guide.builds : [{}];
  return <article className="panel guide-card">
    <header className="guide-card-header">
      <ItemIcon icon={character.icon} fallback={character.name.slice(0, 1)} />
      <div>
        <h2>{character.name}</h2>
        {guide?.patch ? <span className="muted">{tr("Патч ")}{guide.patch}</span> :
          !guide && <span className="muted">{tr("Гайд ещё не заполнен")}</span>}
      </div>
    </header>
    <section className="guide-section">
      <h3>{tr("Роль и особенности")}</h3>
      {guide?.overview ? <p>{guide.overview}</p> : <span className="muted">—</span>}
    </section>
    {builds.map((build, index) => <section className="guide-build" key={index}>
      <h3>{build.title || tr("Сборка")}</h3>
      <div className="guide-sections">
        {guideSections.map(([key, title]) => {
          const entries = build[key];
          return <section key={key} className="guide-section">
            <h4>{title}</h4>
            {entries?.length ? <ul>{entries.map((entry, i) => <li key={i}>{entry}</li>)}</ul> :
              <span className="muted">—</span>}
          </section>;
        })}
      </div>
    </section>)}
  </article>;
}
