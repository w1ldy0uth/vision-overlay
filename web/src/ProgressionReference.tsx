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
  const number = (n: number) => n.toLocaleString("ru-RU");
  const cost = (step: any) => (
    <>
      <span>{number(step.mora)} моры</span>
      {step.materialsAvailable === false ? (
        <p className="muted">Материалы не определены в источнике</p>
      ) : (
        <ul className="progression-materials">
          {step.materials.map((m: any) => (
            <li key={m.id}>
              <ItemIcon
                icon={catalog.items[String(m.id)]?.icon}
                rarity={catalog.rarities[String(m.id)]}
              />
              <span>
                {catalog.names[String(m.id)] ?? `Предмет ${m.id}`} ×{" "}
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
      <summary>Справочник улучшений</summary>
      <p className="footnote">Стоимость каждого перехода указана отдельно.</p>
      <h3>Возвышение {reference.character ? "персонажа" : "оружия"}</h3>
      {reference.promotions.map((step: any) => (
        <details key={step.stage}>
          <summary>
            Этап {step.stage} · ур. {step.requiredLevel} → предел{" "}
            {step.maxLevel}
          </summary>
          <p className="muted">
            Требуется уровень {step.requiredLevel} и ранг приключений{" "}
            {step.requiredAdventureRank}.
          </p>
          {cost(step)}
        </details>
      ))}
      {reference.talents.length > 0 && (
        <>
          <h3>Повышение талантов</h3>
          <p className="footnote">
            Базовые уровни без бонусов созвездий. Для Путешественника показан
            активный набор навыков.
          </p>
          {reference.talents.map((talent: any) => (
            <details key={talent.skillId}>
              <summary>
                {catalog.skillNames[talent.skillId] ??
                  `Навык ${talent.skillId}`}
              </summary>
              {talent.levels.map((step: any) => (
                <details key={step.level}>
                  <summary>
                    Уровень {step.level - 1} → {step.level}
                  </summary>
                  <p className="muted">
                    Требуется этап возвышения {step.requiredAscension}
                    {step.requiredLevel != null
                      ? ` (персонаж от ур. ${step.requiredLevel})`
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
