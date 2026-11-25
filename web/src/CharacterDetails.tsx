import { tr, uiLocale } from "./i18n.mjs";
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
    tr("Базовые характеристики"),
    [
      ["2000", tr("Макс. HP"), false],
      ["2001", tr("Сила атаки"), false],
      ["2002", tr("Защита"), false],
      ["28", tr("Мастерство стихий"), false],
    ],
  ],
  [
    tr("Второстепенные характеристики"),
    [
      ["20", tr("Шанс крит. попадания"), true],
      ["22", tr("Крит. урон"), true],
      ["26", tr("Бонус лечения"), true],
      ["23", tr("Восстановление энергии"), true],
    ],
  ],
  [
    tr("Элементы"),
    [
      ["40", tr("Бонус Пиро"), true],
      ["42", tr("Бонус Гидро"), true],
      ["44", tr("Бонус Анемо"), true],
      ["46", tr("Бонус Крио"), true],
      ["45", tr("Бонус Гео"), true],
      ["41", tr("Бонус Электро"), true],
      ["43", tr("Бонус Дендро"), true],
      ["30", tr("Бонус физ. урона"), true],
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
                <small>{tr("Ур. ")}{r.values.level ?? "—"}
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
            ? tr("Данные экипировки получены не полностью")
            : tr("Не экипировано")}
        </p>
      )}
    </div>
  );
  return (
    <div className="character-details-content">
      {!referenceOnly && (
        <>
          <section>
            <h3>{tr("Оружие")}</h3>
            {equipment("weapons")}
          </section>
          <section>
            <h3>{tr("Артефакты")}</h3>
            {equipment("artifacts")}
          </section>
        </>
      )}
      <section>
        <h3>{tr("Таланты")}</h3>
        {abilities.active.map((t: any) => (
          <article className="ability-card" key={t.id}>
            <AbilityHeading
              entry={t}
              fallback={catalog.skillNames?.[t.id] ?? tr("Навык {0}", t.id)}
            >
              {!referenceOnly && (
                <span className="ability-level">
                  {t.level ? tr("Ур. {0}", t.level.total) : ""}
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
            <AbilityHeading entry={t} fallback={tr("Пассивный талант")} />
            {!referenceOnly && (
              <small className="muted">
                {t.unlocked === true
                  ? tr("Открыт")
                  : t.unlocked === null
                    ? tr("Условия открытия не получены")
                    : tr("Откроется на этапе возвышения {0}", t.requiredAscension)}
              </small>
            )}
          </article>
        ))}
        {!abilities.passives.length && (
          <p className="muted">{tr("Пассивные таланты отсутствуют в текущем справочнике.")}</p>
        )}
      </section>
      <section>
        <h3>{tr("Созвездия")}</h3>
        {abilities.constellations.map((t: any) => (
          <article
            key={t.id}
            className={`ability-card constellation-card${referenceOnly ? " constellation-reference" : t.unlocked ? " constellation-unlocked" : ""}`}
          >
            <AbilityHeading entry={t} fallback={tr("Созвездие {0}", t.position)} />
            <small className={t.unlocked ? "constellation-acquired" : "muted"}>
              {referenceOnly
                ? `C${t.position}`
                : `${t.unlocked ? tr("✓ Получено") : tr("Не получено")} · C${t.position}`}
            </small>
          </article>
        ))}
      </section>
      {!referenceOnly && (
        <section>
          <h3>{tr("Характеристики")}</h3>
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
                              uiLocale,
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
