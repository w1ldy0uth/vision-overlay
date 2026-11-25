import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import assert from "node:assert/strict";

const [catalogPath, russian, english, output, mapSnapshots] = process.argv.slice(2);
assert(catalogPath && russian && english && output,
  "Usage: node scripts/build-english-game-text.mjs CATALOG RUSSIAN_TEXT_MAPS ENGLISH_TEXT_MAPS OUTPUT [MAP_SNAPSHOTS]");
const read = path => JSON.parse(readFileSync(path, "utf8"));
function textMaps(directory, code) {
  const pattern = new RegExp(`^(?:TextMap_)?TextMap(?:_Medium)?${code}(?:_\\d+)?\\.json$`);
  return Object.assign({}, ...readdirSync(directory)
    .filter(name => pattern.test(name)).map(name => read(join(directory, name))));
}
const clean = text => text?.replace(/\{M#([^{}]*)\}/g, "$1").replace(/\{F#[^{}]*\}/g, "")
  .replace(/<\/?(?:color|size|b|i)(?:=[^>]*)?>/g, "").replace(/^#/, "").replaceAll("\\n", "\n")
  .replaceAll("{NON_BREAK_SPACE}", " ").replace(/\{\/?LINK[^}]*\}/g, "");
const ru = textMaps(russian, "RU");
const en = textMaps(english, "EN");
const catalog = read(catalogPath);
const wanted = new Set();
function collect(value) {
  if (typeof value === "string") {
    if (/[А-Яа-яЁё]/.test(value)) wanted.add(value);
  } else if (value && typeof value === "object") {
    for (const entry of Object.values(value)) collect(entry);
  }
}
for (const section of ["names", "avatarNames", "skillNames", "achievements", "equipmentDetails", "characterDetails", "items"])
  collect(catalog[section]);
collect(read(new URL("../web/src/dungeon-schedule.json", import.meta.url)));
const endgame = new URL("../web/public/endgame/", import.meta.url);
for (const file of readdirSync(endgame).filter(name => name.endsWith(".json"))) collect(read(new URL(file, endgame)));
const result = new Map();
const counts = new Set(Object.values(catalog.achievements)
  .flatMap(entry => entry.description?.match(/\d+/g) ?? []));
for (const [hash, value] of Object.entries(ru)) {
  const source = clean(value);
  if (!source || !wanted.has(source) || !en[hash]) continue;
  const target = clean(en[hash]);
  if (target) result.set(source, target);
}
for (const [hash, value] of Object.entries(ru)) {
  if (!value.includes("{param0}") || !en[hash]) continue;
  const source = clean(value);
  const target = clean(en[hash]);
  for (const number of counts) {
    const exact = source.replaceAll("{param0}", number);
    if (wanted.has(exact)) result.set(exact, target.replaceAll("{param0}", number));
  }
}
const matched = result.size;
if (mapSnapshots) {
  function pair(ruRows, enRows, fields) {
    const byId = new Map(enRows.map(row => [row.id, row]));
    for (const row of ruRows) {
      const english = byId.get(row.id);
      if (!english) continue;
      for (const field of fields) if (row[field] && english[field]) result.set(row[field], english[field]);
    }
  }
  for (const id of [2, 7, 9, 34, 36, 37, 40]) {
    const snapshot = (kind, lang) => read(join(mapSnapshots, String(id), `${kind}.${lang}.json`)).data;
    pair([snapshot("map-info", "ru").info], [snapshot("map-info", "en").info], ["name"]);
    const labels = lang => snapshot("labels", lang).tree.flatMap(group => [group, ...group.children]);
    pair(labels("ru"), labels("en"), ["name"]);
    const groups = lang => snapshot("floors", lang).list;
    pair(groups("ru"), groups("en"), ["name"]);
    pair(groups("ru").flatMap(group => group.floors.filter(floor => floor.id)),
      groups("en").flatMap(group => group.floors.filter(floor => floor.id)), ["floor_name"]);
  }
}
assert(result.size > 10000, "Insufficient matching English game text");
writeFileSync(output, JSON.stringify(Object.fromEntries([...result].sort(([a], [b]) => a.localeCompare(b, "ru")))) + "\n");
console.log(`${matched} of ${wanted.size} catalog and schedule strings matched; ${result.size} total with map labels`);
