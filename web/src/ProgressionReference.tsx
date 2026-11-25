import { tr, uiLocale } from "./i18n.mjs";
import { progressionReference } from "./progression.mjs";
import { ItemIcon } from "./InventoryViews";

export function ProgressionReference({
  values,
  catalog,
}: {
  values: any;
  catalog: any;
}) {
  const reference = progressionReference(values, catalog);
  if (!reference) return null;
  const number = (n: number) => n.toLocaleString(uiLocale);
  const cost = (step: any) => (
    <>
      <span>{number(step.mora)}{tr(" моры")}</span>
      {step.materialsAvailable === false ? (
        <p className="muted">{tr("Материалы не определены в источнике")}</p>
      ) : (
        <ul className="progression-materials">
          {step.materials.map((m: any) => (
            <li key={m.id}>
              <ItemIcon
                icon={catalog.items[String(m.id)]?.icon}
                rarity={catalog.rarities[String(m.id)]}
              />
              <span>
                {catalog.names[String(m.id)] ?? tr("Предмет {0}", m.id)} ×{" "}
                {number(m.count)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </>
  );
  return (
    <details className="progression-reference">
      <summary>{tr("Справочник улучшений")}</summary>
      <p className="footnote">{tr("Стоимость каждого перехода указана отдельно.")}</p>
      <h3>{tr("Возвышение ")}{reference.character ? tr("персонажа") : tr("оружия")}</h3>
      {reference.promotions.map((step: any) => (
        <details key={step.stage}>
          <summary>{tr("Этап ")}{step.stage}{tr(" · ур. ")}{step.requiredLevel}{tr(" → предел")}{" "}
            {step.maxLevel}
          </summary>
          <p className="muted">{tr("Требуется уровень ")}{step.requiredLevel}{tr(" и ранг приключений")}{" "}
            {step.requiredAdventureRank}.
          </p>
          {cost(step)}
        </details>
      ))}
      {reference.talents.length > 0 && (
        <>
          <h3>{tr("Повышение талантов")}</h3>
          <p className="footnote">{tr("Базовые уровни без бонусов созвездий. Для Путешественника показан активный набор навыков.")}</p>
          {reference.talents.map((talent: any) => (
            <details key={talent.skillId}>
              <summary>
                {catalog.skillNames[talent.skillId] ??
                  tr("Навык {0}", talent.skillId)}
              </summary>
              {talent.levels.map((step: any) => (
                <details key={step.level}>
                  <summary>{tr("Уровень ")}{step.level - 1} → {step.level}
                  </summary>
                  <p className="muted">{tr("Требуется этап возвышения ")}{step.requiredAscension}
                    {step.requiredLevel != null
                      ? tr(" (персонаж от ур. {0})", step.requiredLevel)
                      : ""}
                    .
                  </p>
                  {cost(step)}
                </details>
              ))}
            </details>
          ))}
        </>
      )}
    </details>
  );
}
