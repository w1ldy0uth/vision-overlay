import { useEffect, useRef, useState } from "react";
import { CharacterGrid, elements } from "./CharacterGrid";
import { InventoryGrid } from "./InventoryViews";
import { characterElement } from "./characters.mjs";
import { itemSubtype, subtypeOptions } from "./inventory.mjs";
import { characterLevel } from "./model.mjs";
import avatarRarities from "./data/wish-avatar-rarities.json";
import type { Row } from "./api";

export type GoalChoice = Row & { collection: "characters" | "weapons" };

function rarityOf(row: GoalChoice, catalog: any): number {
  const { avatar_id, item_id } = row.values;
  if (row.collection === "weapons") {
    return Number(catalog.rarities[item_id] ?? catalog.items?.[item_id]?.rarity ?? 0);
  }
  const quality = catalog.avatarMetadata?.[avatar_id]?.qualityType;
  if (quality === "QUALITY_PURPLE") return 4;
  if (quality === "QUALITY_ORANGE" || quality === "QUALITY_ORANGE_SP") return 5;
  return Number((avatarRarities.rows as Record<string, number>)[avatar_id] ?? 0);
}

export function GoalPicker({ choices, catalog, characters, name, onSelect, onClose }: {
  choices: GoalChoice[];
  catalog: any;
  characters: Row[];
  name: (values: Row["values"]) => string;
  onSelect: (row: GoalChoice) => void;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [collection, setCollection] = useState<GoalChoice["collection"]>("characters");
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState("name");
  const [type, setType] = useState("");
  const [rarity, setRarity] = useState("");
  useEffect(() => {
    const node = dialog.current!;
    node.showModal();
    return () => node.close();
  }, []);
  const isCharacter = collection === "characters";
  const items = catalog.items ?? {};
  const candidates = choices.filter(row => row.collection === collection);
  const typeOptions = isCharacter ? Object.entries(elements).map(([id, [label]]) => [id, label]) :
    subtypeOptions(candidates, items).map(([label]: [string, number]) => [label, label]);
  const rarityOptions = [...new Set(candidates.map(row => rarityOf(row, catalog)))].filter(Boolean).sort((a, b) => b - a);
  const level = (row: GoalChoice) => isCharacter ? characterLevel(row.values) : Number(row.values.level ?? 0);
  const needle = search.trim().toLocaleLowerCase("ru");
  const rows = candidates.filter(row =>
    name(row.values).toLocaleLowerCase("ru").includes(needle) &&
    (!type || (isCharacter ? characterElement(row.values, catalog) : itemSubtype(row, items)) === type) &&
    (!rarity || rarityOf(row, catalog) === Number(rarity))
  ).sort((a, b) => (sort === "level" ? level(b) - level(a) : 0) || name(a.values).localeCompare(name(b.values), "ru"));

  return <dialog ref={dialog} className="goal-picker" aria-labelledby="goal-picker-title"
    onCancel={e => { e.preventDefault(); onClose(); }}>
    <div className="goal-picker-toolbar">
      <div className="goal-picker-heading">
        <h2 id="goal-picker-title">Добавить цель</h2>
        <button type="button" aria-label="Закрыть выбор цели" onClick={onClose}>×</button>
      </div>
      <div className="goal-picker-tabs" aria-label="Виды целей">
        {(["characters", "weapons"] as const).map(id => <button key={id} type="button"
          aria-pressed={collection === id}
          onClick={() => { setCollection(id); setType(""); setRarity(""); }}>
          {id === "characters" ? "Персонажи" : "Оружие"}
        </button>)}
      </div>
      <div className="goal-picker-filters">
        <input aria-label="Поиск по названию" placeholder="Поиск по названию" autoFocus value={search} onChange={e => setSearch(e.target.value)} />
        <select aria-label="Сортировка" value={sort} onChange={e => setSort(e.target.value)}>
          <option value="name">По названию</option><option value="level">По уровню</option>
        </select>
        <select aria-label={isCharacter ? "Стихия" : "Тип оружия"} value={type} onChange={e => setType(e.target.value)}>
          <option value="">{isCharacter ? "Все стихии" : "Все типы"}</option>
          {typeOptions.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
        </select>
        <select aria-label="Редкость" value={rarity} onChange={e => setRarity(e.target.value)}>
          <option value="">Любая редкость</option>
          {rarityOptions.map(value => <option key={value} value={value}>{value} ★</option>)}
        </select>
      </div>
    </div>
    {!rows.length ? <p role="status">{candidates.length ? "Ничего не найдено." : "Нет доступных целей."}</p> :
      isCharacter ? <CharacterGrid rows={rows} catalog={catalog} name={name} favorites={[]} onSelect={onSelect} rarity={(row: GoalChoice) => rarityOf(row, catalog)} /> :
        <InventoryGrid rows={rows.map(row => ({ ...row, source_collection: "weapons" }))} items={items}
          rarities={catalog.rarities} characters={characters} avatarIcons={catalog.avatarIcons ?? {}} name={name}
          onSelect={row => onSelect({ ...row, collection: "weapons" })} />}
  </dialog>;
}
