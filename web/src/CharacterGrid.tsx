import { ItemIcon } from "./InventoryViews";
import { characterElement } from "./characters.mjs";
import { characterLevel, characterConstellation } from "./model.mjs";
export const elements: Record<string, [string, string]> = {
  Fire: ["Пиро", "♨"],
  Water: ["Гидро", "◉"],
  Wind: ["Анемо", "❧"],
  Ice: ["Крио", "❄"],
  Rock: ["Гео", "◆"],
  Electric: ["Электро", "ϟ"],
  Grass: ["Дендро", "❀"],
};
export function CharacterGrid({
  rows,
  catalog,
  name,
  favorites,
  toggleFavorite,
  onSelect,
  rarity,
}: any) {
  return (
    <div className="character-grid">
      {rows.map((r: any) => {
        const id = r.values.avatar_id,
          starred = favorites.includes(id),
          element = characterElement(r.values, catalog),
          label = elements[element];
        return (
          <article
            className={`character character-tile${r.unowned ? " character-unowned" : ""}`}
            key={r.entity_key}
          >
            {label && (
              <span
                className={`character-element element-${element}`}
                title={label[0]}
                aria-label={`Стихия: ${label[0]}`}
              >
                {element === "Ice" ? (
                  <svg
                    className="cryo-icon"
                    viewBox="0 0 32 32"
                    aria-hidden="true"
                  >
                    <g
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1.8"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    >
                      {[0, 60, 120, 180, 240, 300].map((angle) => (
                        <path
                          key={angle}
                          transform={`rotate(${angle} 16 16)`}
                          d="M16 16V3 M16 8L11.5 5 M16 8L20.5 5 M16 12L12 9.5 M16 12L20 9.5"
                        />
                      ))}
                    </g>
                  </svg>
                ) : (
                  <ItemIcon
                    icon={`UI_Buff_Element_${element}`}
                    fallback={label[1]}
                  />
                )}
              </span>
            )}
            {!r.unowned && toggleFavorite && (
              <button
                type="button"
                className={`character-favorite${starred ? " selected" : ""}`}
                aria-label={`${starred ? "Убрать из избранного" : "В избранное"}: ${name(r.values)}`}
                aria-pressed={starred}
                onClick={() => toggleFavorite(id)}
              >
                {starred ? "★" : "☆"}
              </button>
            )}
            <button
              type="button"
              className="character-open"
              aria-label={
                r.unowned
                  ? `${name(r.values)} — не получен`
                  : `Открыть персонажа ${name(r.values)}`
              }
              onClick={() => onSelect(r)}
            >
              <ItemIcon
                icon={catalog.avatarIcons?.[id]}
                fallback={name(r.values).slice(0, 1)}
                rarity={rarity?.(r)}
                large
              />
              <h3>{name(r.values)}</h3>
              {rarity?.(r) > 0 && <span className="inventory-card-stars">{"★".repeat(rarity(r))}</span>}
              {r.unowned ? (
                <small>Не получен</small>
              ) : (
                <>
                  <span>
                    Ур. {characterLevel(r.values)}{" "}
                    <b>C{characterConstellation(r.values)}</b>
                  </span>
                  <small>Дружба {r.values.friendship?.level ?? "—"}</small>
                </>
              )}
            </button>
          </article>
        );
      })}
    </div>
  );
}
