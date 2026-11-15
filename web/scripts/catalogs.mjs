// Bootstrap is produced from the pinned, validated bundle by the Rust build helper.
// Copy bytes verbatim: JavaScript must not round arbitrary-precision game values.
import fs from "node:fs";
import path from "node:path";
const source = path.resolve(import.meta.dirname, "../../local-data/build-input/catalog.payload.json");
if (!fs.existsSync(source)) throw new Error("Prepared catalog missing. Run scripts/Build-Web.ps1 first.");
const raw = fs.readFileSync(source);
const catalog = JSON.parse(raw);
for (const name of ["characterDetails", "equipmentDetails", "progression", "leveling", "crafting"])
  if (!catalog[name]) throw new Error(`Incomplete bootstrap: ${name}`);
fs.mkdirSync("public", { recursive: true });
fs.writeFileSync("public/catalog.json", raw);
console.log(`Pinned catalog: ${Object.keys(catalog.names).length} item names`);
