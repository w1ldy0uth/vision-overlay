import { tr, uiLocale } from "./i18n.mjs";
import { nextUpgrades } from "./upgrades.mjs";
import { ItemIcon } from "./InventoryViews";

const number = (n: number) => n.toLocaleString(uiLocale);
const status: Record<string, string> = {
  ready: tr("Ресурсов достаточно"),
  saved: tr("Достаточно по сохранённым данным"),
  shortage: tr("Не хватает ресурсов по полученным данным"),
  unknown: tr("Запасы подтверждены не полностью"),
  locked: tr("Сначала выполните требования"),
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
      ? tr("Возвышение {0} → {1}", s.from, s.to)
      : s.kind === "talent"
        ? tr("Базовый уровень {0} → {1}", s.from, s.to)
        : tr("Уровень {0} → {1}", s.from, s.to);
  const itemName = (id: number) => catalog.names[String(id)] ?? tr("Предмет {0}", id);
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
                {" "}{tr("· есть ")}{m.owned === null ? tr("неизвестно") : number(m.owned)}
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
        <p className="muted">{tr("Уровень от ")}{s.requiredLevel}{tr(", ранг приключений от")}{" "}
          {s.requiredAdventureRank}{tr("; новый предел — ")}{s.maxLevel}.
        </p>
      )}
      {s.kind === "talent" && (
        <p className="muted">{tr("Нужно возвышение ")}{s.requiredAscension}{tr(", уровень от")}{" "}
          {s.requiredLevel ?? tr("неизвестен")}{tr(". Бонусы созвездий в цену не входят.")}</p>
      )}
      {s.kind === "level" && (
        <>
          <p>{tr("Опыт: ")}{number(s.currentExp)} / {number(s.requiredExp)}{tr(" · до следующего уровня ")}{number(s.neededExp)}.
          </p>
          <p className="muted">
            {s.fromStock
              ? tr("Вариант из полученных запасов")
              : tr("Пример расхода материалов")}
            : {number(s.suppliedExp)}{tr(" опыта. Результат: ур. ")}{s.resultLevel}
            {s.resultExp ? tr(", опыт {0}", number(s.resultExp)) : ""}.
          </p>
        </>
      )}
      <p>
        <strong>{number(s.mora)}{tr(" моры")}</strong>
        <small className="muted">
          {" "}{tr("· есть ")}{s.moraOwned === null ? tr("неизвестно") : number(s.moraOwned)}
        </small>
      </p>
      {s.materialsAvailable === false ? (
        <p className="muted">{tr("Материалы не определены в источнике.")}</p>
      ) : (
        materials(s.materials)
      )}
      {!!s.returnedMaterials?.length && (
        <>
          <p>{tr("Возврат после достижения предела:")}</p>
          {materials(s.returnedMaterials)}
        </>
      )}
      {!!s.lostExp && (
        <p className="footnote">{tr("На пределе не будет использовано ")}{number(s.lostExp)}{tr(" опыта.")}</p>
      )}
    </>
  );
  return (
    <section className="next-upgrades">
      {!only && <h3>{tr("Следующие улучшения")}</h3>}
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
                  ? tr("Следующее улучшение")
                  : (catalog.skillNames[t.skillId] ??
                    tr("Навык {0}", t.skillId))}{" "}
                ·{" "}
                {t.kind === "max"
                  ? tr("максимум")
                  : t.kind === "unknown"
                    ? tr("нет данных")
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
        <p className="footnote">{tr("Расчёт по последним данным профиля, без бонусов событий. Для повышения уровня предложены книги или руда; оружие в качестве материала не расходуется.")}</p>
      )}
    </section>
  );
}
