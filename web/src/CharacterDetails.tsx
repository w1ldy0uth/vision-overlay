import React, { useState, useId, useEffect, useRef } from "react";
import { TooltipButton } from "./TooltipButton";
import { ArtifactStatsInline } from "./EquipmentDetails";
import { equipmentTooltip } from "./equipment-details.mjs";
import { ItemIcon } from "./InventoryViews";
import { NextUpgrades } from "./NextUpgrades";
import { characterAbilities } from "./character-details.mjs";
function AbilityHeading({ entry, fallback, children }: any) {
  return (
    <div className="ability-heading">
      <TooltipButton
        entry={entry}
        fallback={fallback}
        className="ability-description"
      >
        <ItemIcon icon={entry.icon} fallback="◇" />
        <span>{entry.name ?? fallback}</span>
      </TooltipButton>
      {children}
    </div>
  );
}
const stats = [
  [
    "Базовые характеристики",
    [
      ["2000", "Макс. HP", false],
      ["2001", "Сила атаки", false],
      ["2002", "Защита", false],
      ["28", "Мастерство стихий", false],
    ],
  ],
  [
    "Второстепенные характеристики",
    [
      ["20", "Шанс крит. попадания", true],
      ["22", "Крит. урон", true],
      ["26", "Бонус лечения", true],
      ["23", "Восстановление энергии", true],
    ],
  ],
  [
    "Элементы",
    [
      ["40", "Бонус Пиро", true],
      ["42", "Бонус Гидро", true],
      ["44", "Бонус Анемо", true],
      ["46", "Бонус Крио", true],
      ["45", "Бонус Гео", true],
      ["41", "Бонус Электро", true],
      ["43", "Бонус Дендро", true],
      ["30", "Бонус физ. урона", true],
    ],
  ],
] as const;
export function CharacterDetails({
  values: v,
  catalog,
  data,
  name,
  openItem,
  referenceOnly = false,
}: any) {
  const abilities = characterAbilities(v, catalog, { referenceOnly });
  const equipped = (v.equip_guids ?? [])
    .map((guid: string) => {
      for (const collection of ["weapons", "artifacts"]) {
        const row = data.collections[collection]?.find(
          (r: any) => r.entity_key === guid,
        );
        if (row) return { ...row, source_collection: collection };
      }
      return null;
    })
    .filter(Boolean);
  const equipment = (collection: string) => (
    <div className="character-equipment-list">
      {equipped
        .filter((r: any) => r.source_collection === collection)
        .map((r: any) => (
          <TooltipButton
            key={r.entity_key}
            entry={equipmentTooltip(r, equipped, catalog)}
            className="character-equipment-link"
            onClick={() => openItem(r)}
          >
            <ItemIcon
              icon={catalog.items?.[r.values.item_id]?.icon}
              rarity={catalog.rarities?.[r.values.item_id]}
            />
            <span>
              <strong>{name(r.values)}</strong>
              {collection === "artifacts" ? (
                <ArtifactStatsInline values={r.values} catalog={catalog} />
              ) : (
                <small>
                  Ур. {r.values.level ?? "—"}
                  {r.values.refinement !== undefined
                    ? ` · R${r.values.refinement}`
                    : ""}
                </small>
              )}
            </span>
            <span aria-hidden="true">›</span>
          </TooltipButton>
        ))}
      {!equipped.some((r: any) => r.source_collection === collection) && (
        <p className="muted">
          {equipped.length < (v.equip_guids ?? []).length
            ? "Данные экипировки получены не полностью"
            : "Не экипировано"}
        </p>
      )}
    </div>
  );
  return (
    <div className="character-details-content">
      {!referenceOnly && (
        <>
          <section>
            <h3>Оружие</h3>
            {equipment("weapons")}
          </section>
          <section>
            <h3>Артефакты</h3>
            {equipment("artifacts")}
          </section>
        </>
      )}
      <section>
        <h3>Таланты</h3>
        {abilities.active.map((t: any) => (
          <article className="ability-card" key={t.id}>
            <AbilityHeading
              entry={t}
              fallback={catalog.skillNames?.[t.id] ?? `Навык ${t.id}`}
            >
              {!referenceOnly && (
                <span className="ability-level">
                  {t.level ? `Ур. ${t.level.total}` : ""}
                  {t.level?.bonus > 0 && (
                    <small>
                      {t.level.base} + {t.level.bonus}
                    </small>
                  )}
                </span>
              )}
            </AbilityHeading>
            {!referenceOnly && t.level && (
              <NextUpgrades
                values={v}
                catalog={catalog}
                data={data}
                only="talent"
                talentId={t.id}
              />
            )}
          </article>
        ))}
        {abilities.passives.map((t: any) => (
          <article className="ability-card" key={t.id}>
            <AbilityHeading entry={t} fallback="Пассивный талант" />
            {!referenceOnly && (
              <small className="muted">
                {t.unlocked === true
                  ? "Открыт"
                  : t.unlocked === null
                    ? "Условия открытия не получены"
                    : `Откроется на этапе возвышения ${t.requiredAscension}`}
              </small>
            )}
          </article>
        ))}
        {!abilities.passives.length && (
          <p className="muted">
            Пассивные таланты отсутствуют в текущем справочнике.
          </p>
        )}
      </section>
      <section>
        <h3>Созвездия</h3>
        {abilities.constellations.map((t: any) => (
          <article
            key={t.id}
            className={`ability-card constellation-card${referenceOnly ? " constellation-reference" : t.unlocked ? " constellation-unlocked" : ""}`}
          >
            <AbilityHeading entry={t} fallback={`Созвездие ${t.position}`} />
            <small className={t.unlocked ? "constellation-acquired" : "muted"}>
              {referenceOnly
                ? `C${t.position}`
                : `${t.unlocked ? "✓ Получено" : "Не получено"} · C${t.position}`}
            </small>
          </article>
        ))}
      </section>
      {!referenceOnly && (
        <section>
          <h3>Характеристики</h3>
          {stats.map(([title, fields]) => (
            <section className="character-stat-group" key={title}>
              <h4>{title}</h4>
              <dl>
                {fields.map(([id, label, percent]) => (
                  <React.Fragment key={id}>
                    <dt>{label}</dt>
                    <dd>
                      {v.fight_props?.[id] === undefined
                        ? "—"
                        : percent
                          ? `${(v.fight_props[id] * 100).toFixed(1)}%`
                          : Math.round(v.fight_props[id]).toLocaleString(
                              "ru-RU",
                            )}
                    </dd>
                  </React.Fragment>
                ))}
              </dl>
            </section>
          ))}
        </section>
      )}
    </div>
  );
}
