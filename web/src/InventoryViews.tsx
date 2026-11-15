import React, { createContext, useContext, useEffect, useState } from "react";
import { equipmentOwners } from "./equipment-owners.mjs";
import type { Row } from "./api";
export const CatalogIconVersion = createContext("");

export function ItemIcon({
  icon,
  rarity = 0,
  fallback = "◇",
  large = false,
}: {
  icon?: string;
  rarity?: number;
  fallback?: string;
  large?: boolean;
}) {
  const [failed, setFailed] = useState(false);
  const [attempt,setAttempt]=useState(0);
  const version = useContext(CatalogIconVersion);
  useEffect(() => {setFailed(false);setAttempt(0);}, [icon, version]);
  useEffect(()=>{if(!failed||!icon||attempt>=2)return;const timer=setTimeout(()=>{setAttempt(n=>n+1);setFailed(false);},(attempt+1)*60000);return()=>clearTimeout(timer);},[failed,icon,attempt]);
  return (
    <span
      title={!icon||failed?"Изображение недоступно":undefined}
      className={`item-icon rarity-${rarity}${large ? " item-icon-large" : ""}`}
    >
      {icon && !failed ? (
        <img
          src={`/icons/${encodeURIComponent(icon)}?${new URLSearchParams({...((version&&version!=="bundled")?{v:version}:{}),...(attempt?{retry:String(attempt)}:{})})}`}
          alt=""
          loading="lazy"
          decoding="async"
          onError={() => setFailed(true)}
        />
      ) : (
        <span aria-hidden="true">{fallback}</span>
      )}
    </span>
  );
}
export function InventoryGrid({
  rows,
  items,
  rarities,
  characters,
  avatarIcons,
  name,
  onSelect,
}: {
  rows: Row[];
  items: Record<string, any>;
  rarities: Record<string, number>;
  characters: Row[];
  avatarIcons: Record<string,string>;
  name: (v: Record<string, any>) => string;
  onSelect: (row: Row) => void;
}) {
  const owners = equipmentOwners(characters);
  return (
    <div className="inventory-grid">
      {rows.map((row) => {
        const id = String(row.values.item_id),
          v = row.values;
        const rarity = rarities[id] ?? items[id]?.rarity ?? 0;
        const equipment = ["weapons", "artifacts"].includes(
          row.source_collection ?? "",
        );
        const owner = equipment ? owners.get(row.entity_key) : null;
        const count = equipment
          ? `Ур. ${v.level ?? "—"}`
          : `${Number(v.count ?? 0).toLocaleString("ru-RU")} шт.`;
        return (
          <button
            className="inventory-card"
            key={`${row.source_collection}:${row.entity_key}`}
            onClick={() => onSelect(row)}
            aria-label={`${name(v)}, ${count}${owner ? `, экипировано: ${name(owner.values)}` : ""}`}
          >
            <ItemIcon icon={items[id]?.icon} rarity={rarity} fallback={name(v).slice(0,1)} large />
            {owner && <span className="equipment-owner-badge" title={`Экипировано: ${name(owner.values)}`}><ItemIcon icon={avatarIcons[owner.values.avatar_id]} fallback={name(owner.values).slice(0,1)} /></span>}
            <span className="inventory-card-count">
              {count}
              {row.source_collection === "weapons"
                ? ` · R${v.refinement ?? "—"}`
                : ""}
            </span>
            <span className="inventory-card-name">{name(v)}</span>
            <span className="inventory-card-stars">{"★".repeat(rarity)}</span>
            {row.confirmation !== "observed" && (
              <small className="inventory-card-old">Из прошлых данных</small>
            )}
          </button>
        );
      })}
    </div>
  );
}
