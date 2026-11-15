import { nextUpgrades } from "./upgrades.mjs";
import { ItemIcon } from "./InventoryViews";

const number = (n: number) => n.toLocaleString("ru-RU");
const status: Record<string, string> = {
  ready: "Ресурсов достаточно",
  saved: "Достаточно по сохранённым данным",
  shortage: "Не хватает ресурсов по полученным данным",
  unknown: "Запасы подтверждены не полностью",
  locked: "Сначала выполните требования",
};
export function NextUpgrades({
  values,
  catalog,
  data,
  only,
  talentId,
}: {
  values: any;
  catalog: any;
  data: any;
  only?: "progress" | "talent";
  talentId?: string;
}) {
  const next = nextUpgrades(values, catalog, data);
  if (!next) return null;
  const title = (s: any) =>
    s.kind === "ascension"
      ? `Возвышение ${s.from} → ${s.to}`
      : s.kind === "talent"
        ? `Базовый уровень ${s.from} → ${s.to}`
        : `Уровень ${s.from} → ${s.to}`;
  const itemName = (id: number) => catalog.names[String(id)] ?? `Предмет ${id}`;
  const materials = (items: any[]) => (
    <ul className="progression-materials">
      {items.map((m: any) => (
        <li key={m.id}>
          <ItemIcon
            icon={catalog.items[String(m.id)]?.icon}
            rarity={catalog.rarities[String(m.id)]}
          />
          <span>
            {itemName(m.id)} × {number(m.count)}
            {m.owned !== undefined && (
              <small className="muted">
                {" "}
                · есть {m.owned === null ? "неизвестно" : number(m.owned)}
              </small>
            )}
          </span>
        </li>
      ))}
    </ul>
  );
  const quote = (s: any) => (
    <>
      <p className={`upgrade-status ${s.status}`}>{status[s.status]}</p>
      {s.blockers.map((reason: string) => (
        <p className="muted" key={reason}>
          {reason}
        </p>
      ))}
      {s.kind === "ascension" && (
        <p className="muted">
          Уровень от {s.requiredLevel}, ранг приключений от{" "}
          {s.requiredAdventureRank}; новый предел — {s.maxLevel}.
        </p>
      )}
      {s.kind === "talent" && (
        <p className="muted">
          Нужно возвышение {s.requiredAscension}, уровень от{" "}
          {s.requiredLevel ?? "неизвестен"}. Бонусы созвездий в цену не входят.
        </p>
      )}
      {s.kind === "level" && (
        <>
          <p>
            Опыт: {number(s.currentExp)} / {number(s.requiredExp)} · до
            следующего уровня {number(s.neededExp)}.
          </p>
          <p className="muted">
            {s.fromStock
              ? "Вариант из полученных запасов"
              : "Пример расхода материалов"}
            : {number(s.suppliedExp)} опыта. Результат: ур. {s.resultLevel}
            {s.resultExp ? `, опыт ${number(s.resultExp)}` : ""}.
          </p>
        </>
      )}
      <p>
        <strong>{number(s.mora)} моры</strong>
        <small className="muted">
          {" "}
          · есть {s.moraOwned === null ? "неизвестно" : number(s.moraOwned)}
        </small>
      </p>
      {s.materialsAvailable === false ? (
        <p className="muted">Материалы не определены в источнике.</p>
      ) : (
        materials(s.materials)
      )}
      {!!s.returnedMaterials?.length && (
        <>
          <p>Возврат после достижения предела:</p>
          {materials(s.returnedMaterials)}
        </>
      )}
      {!!s.lostExp && (
        <p className="footnote">
          На пределе не будет использовано {number(s.lostExp)} опыта.
        </p>
      )}
    </>
  );
  return (
    <section className="next-upgrades">
      {!only && <h3>Следующие улучшения</h3>}
      {only !== "talent" &&
        (["max", "unknown"].includes(next.progress.kind) ? (
          <p className="muted">{next.progress.reason}</p>
        ) : (
          <details open>
            <summary>{title(next.progress)}</summary>
            {quote(next.progress)}
          </details>
        ))}
      {only !== "progress" &&
        next.talents
          .filter((t: any) => !talentId || String(t.skillId) === talentId)
          .map((t: any) => (
            <details key={t.skillId}>
              <summary>
                {only === "talent"
                  ? "Следующее улучшение"
                  : (catalog.skillNames[t.skillId] ??
                    `Навык ${t.skillId}`)}{" "}
                ·{" "}
                {t.kind === "max"
                  ? "максимум"
                  : t.kind === "unknown"
                    ? "нет данных"
                    : `${t.from} → ${t.to}`}
              </summary>
              {["max", "unknown"].includes(t.kind) ? (
                <p className="muted">{t.reason}</p>
              ) : (
                quote(t)
              )}
            </details>
          ))}
      {!only && (
        <p className="footnote">
          Расчёт по последним данным профиля, без бонусов событий. Для повышения
          уровня предложены книги или руда; оружие в качестве материала не
          расходуется.
        </p>
      )}
    </section>
  );
}
