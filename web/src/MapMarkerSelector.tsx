import { tr, uiLocale } from "./i18n.mjs";
import { useRef, useState } from "react";
import type { MapSource, WorldMapData } from "./world-map";

export function MapMarkerSelector({ categories, sources, hidden, show }: {
  categories: WorldMapData["categories"];
  sources: MapSource[];
  hidden: ReadonlySet<number>;
  show: (sources: MapSource[], visible: boolean) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [category, setCategory] = useState(categories[0]?.id);
  const [search, setSearch] = useState("");
  const groups = categories.map(entry => ({ ...entry, entries: sources.filter(source => source.category === entry.id) }));
  const current = groups.find(group => group.id === category) ?? groups[0];
  const query = search.trim().toLocaleLowerCase(uiLocale);
  const visible = (query ? sources : current?.entries ?? []).filter(source => source.name.toLocaleLowerCase(uiLocale).includes(query));
  const enabled = sources.filter(source => !hidden.has(source.id)).length;

  return <>
    <button type="button" aria-haspopup="dialog" onClick={() => dialog.current?.showModal()}>{tr("Категории меток · ")}{enabled}/{sources.length} ▾
    </button>
    <dialog ref={dialog} className="map-marker-dialog" aria-labelledby="map-marker-dialog-title"
      onClick={event => { if (event.target === event.currentTarget) dialog.current?.close(); }}>
      <div className="map-marker-dialog-content">
        <div className="map-marker-dialog-header">
          <h2 id="map-marker-dialog-title">{tr("Категории меток")}</h2>
          <button type="button" aria-label={tr("Закрыть селектор меток")} onClick={() => dialog.current?.close()}>×</button>
        </div>
        <div className="world-map-filter-actions">
          <input type="search" autoFocus aria-label={tr("Поиск меток")} placeholder={tr("Поиск меток")} value={search}
            onChange={event => setSearch(event.target.value)} />
          <button type="button" onClick={() => show(sources, true)}>{tr("Показать все")}</button>
          <button type="button" onClick={() => show(sources, false)}>{tr("Скрыть все")}</button>
        </div>
        <div className="map-marker-picker">
          <div className="map-marker-groups" aria-label={tr("Группы меток")}>
            {groups.map(group => {
              const count = group.entries.filter(source => !hidden.has(source.id)).length;
              return <div key={group.id} className="map-marker-group">
                <input type="checkbox" aria-label={group.name} checked={count === group.entries.length}
                  ref={input => { if (input) input.indeterminate = count > 0 && count < group.entries.length; }}
                  onChange={event => show(group.entries, event.target.checked)} />
                <button type="button" aria-pressed={!query && current?.id === group.id} onClick={() => {
                  setCategory(group.id);
                  setSearch("");
                }}>
                  <MarkerIcon source={group.entries[0]} />
                  <span>{group.name}</span>
                  <small className="muted">{count}/{group.entries.length}</small>
                </button>
              </div>;
            })}
          </div>
          <div className="map-marker-entries" aria-label={query ? tr("Результаты поиска") : current?.name}>
            {visible.length ? visible.map(source => <label key={source.id}>
              <input type="checkbox" checked={!hidden.has(source.id)}
                onChange={event => show([source], event.target.checked)} />
              <MarkerIcon source={source} />
              <span>{source.name}</span>
            </label>) : <p className="muted">{tr("Метки не найдены.")}</p>}
          </div>
        </div>
      </div>
    </dialog>
  </>;
}

function MarkerIcon({ source }: { source: MapSource }) {
  return <img src={source.icon} alt="" referrerPolicy="no-referrer" loading="lazy" draggable={false} width="24" height="24" />;
}
