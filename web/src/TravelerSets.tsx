import { tr } from "./i18n.mjs";
import { travelerSets } from "./traveler.mjs";
import { Fragment } from "react";
export function TravelerSets({
  values,
  catalog,
  data,
}: {
  values: any;
  catalog: any;
  data: any;
}) {
  const sets = travelerSets(values, catalog, data);
  if (!sets.length) return null;
  return (
    <details className="traveler-sets">
      <summary>{tr("Стихии Путешественника · получено")}{" "}
        {sets.filter((s: any) => s.known).length}{tr(" из ")}{sets.length}
      </summary>
      <p className="footnote">{tr("Таланты и созвездия сохранены отдельно для каждой стихии. Чтобы получить неизвестный набор, переключитесь на эту стихию в игре. Уровень и экипировка общие.")}</p>
      {sets.map((set: any) => (
        <details key={set.id}>
          <summary>
            {set.name}
            {set.active ? tr(" · активная") : ""}
            {set.known ? ` · C${set.constellation}` : tr(" · нет данных")}
          </summary>
          {set.known ? (
            <>
              {set.saved && (
                <p className="muted">{tr("Сохранённый набор, в текущей сессии ещё не получен.")}</p>
              )}
              <dl>
                {set.talents.map((t: any) => (
                  <Fragment key={t.id}>
                    <dt>{catalog.skillNames[t.id] ?? tr("Навык {0}", t.id)}</dt>
                    <dd>
                      {t.total}
                      {t.bonus > 0 ? ` (${t.base} + ${t.bonus})` : ""}
                    </dd>
                  </Fragment>
                ))}
              </dl>
            </>
          ) : (
            <p className="muted">{tr("Этот набор ещё не наблюдался в захвате.")}</p>
          )}
        </details>
      ))}
    </details>
  );
}
