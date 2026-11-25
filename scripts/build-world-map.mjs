import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [directory, gameVersion, output] = process.argv.slice(2);
assert(directory && /^\d+\.\d+\.\d+$/.test(gameVersion ?? "") && output,
  "Usage: node scripts/build-world-map.mjs SNAPSHOT_DIRECTORY GAME_VERSION OUTPUT");
// Russian HoYoLAB public responses for a single map_id with lang=ru-ru.
function read(name) {
  const response = JSON.parse(readFileSync(join(directory, name), "utf8"));
  assert.equal(response.retcode, 0, name);
  return response.data;
}
const { id: mapId, name: mapName, detail_v2: detail } = read("map-info.ru.json").info;
const { map_version: mapVersion, origin, total_size: totalSize, min_zoom: minZoom, max_zoom: maxZoom } = detail;
const labels = read("labels.ru.json").tree;
const points = read("map-points.ru.json").point_list;
assert(/^[a-f0-9]+$/.test(mapVersion));

const iconPaths = new Map([
  ["act-webstatic.hoyoverse.com", "/map_manage/"],
  ["upload-static.hoyoverse.com", "/map_manage/"],
  ["act.hoyoverse.com", "/map_manage/"],
  ["uploadstatic.hoyoverse.com", "/ys-obc/"],
]);
const sources = new Map(labels.flatMap(group => group.children.map(label => {
  const { id, name, icon, item_id: item } = label;
  const url = new URL(icon);
  const path = iconPaths.get(url.host);
  assert(url.protocol === "https:" && path && url.pathname.startsWith(path), `Unexpected map icon ${icon}`);
  return [id, { id, category: group.id, kind: "object", name, icon, items: item ? [item] : [], points: [] }];
})));

// Map labels use their own IDs. Keys below are map labels, values are game items.
const enemies = [
  [[55], 112005], [[57], 112008],
  [[58], 112035], [[56], 112011],
  [[59], 112002], [[209], 112044],
  [[229], 112053], [[54], 112032],
  [[47, 48, 213], 112038], [[221], [112059,112062]],
  [[389], 112065], [[493], 112080],
  [[494], 112083], [[603], 112101],
  [[606], 112104], [[738], 112122],
  [[739], 112125], [[149,53], 112014],
  [[24], 112020], [[25], 112029],
  [[26], 112026], [[215], 112047],
  [[150,27,28], 112023],
  [[49,169], 112041], [[210], 112050],
  [[265], 112056], [[337], 112020],
  [[344], 112017], [[434], 112071],
  [[388], 112068], [[454], 112074],
  [[456], 112077], [[495], 112086],
  [[496], 112089], [[541], 112092],
  [[563], 112095], [[585], 112098],
  [[605], 112113],
  [[607], 112110], [[604], 112107],
  [[612], 112116], [[669], 112119],
  [[736], 112131], [[737], 112128],
  [[740], 112134], [[768], 112137],
  [[809], 112140], [[843], 112152],
  [[844], 112146], [[845], 112149],
  [[846], 112155], [[881], 112158],
];
const bosses = {
  132:113001, 133:113010, 135:113002, 136:113012, 137:113011, 138:113009,
  157:113016, 181:113020, 183:113022, 203:113024, 204:113023, 262:113028,
  263:113029, 318:113030, 333:113031, 352:113035, 390:113037, 391:113036,
  432:113038, 433:113039, 452:113040, 457:113044, 455:113045, 497:[113049,113050],
  498:113051, 539:113052, 540:113053, 553:113057, 564:113058, 586:113059,
  609:113064, 610:113065, 608:113066, 629:113067, 660:113071, 670:113072,
  687:113076, 743:113077, 744:113078, 762:113079, 764:113080,
  769:113084, 788:113085, 810:113086, 345:113090, 222:113091, 823:113092, 878:113093,
};
function linkDrops(id, kind, items) {
  const source = sources.get(id);
  if (!source && mapId !== 2) return;
  assert(source && source.kind === "object", `Invalid or duplicate map label ${id}`);
  source.kind = kind;
  source.items = [...new Set([...source.items, ...[items].flat()])];
}
for (const [labels, tiers] of enemies) {
  const items = [tiers].flat().flatMap(id => [id, id + 1, id + 2]);
  for (const label of labels) linkDrops(label, "enemy", items);
}
for (const [label, items] of Object.entries(bosses)) linkDrops(Number(label), "boss", items);
for (const source of sources.values()) {
  if (source.kind === "object" && [10, 11, 60, 173].includes(source.category)) source.kind = "resource";
}
const pointById = new Map(points.map(point => [point.id, point]));
const floors = new Set();
const floorGroups = read("floors.ru.json").list.flatMap(group => {
  const regions = new Map();
  const levels = group.floors.flatMap(floor => {
    const { id, floor_name, floor_name_short, overlay } = floor;
    if (!id || !overlay?.url) return [];
    const url = new URL(overlay.url);
    assert(url.protocol === "https:" && ["act-upload.hoyoverse.com", "act.hoyoverse.com"].includes(url.host)
      && url.pathname.startsWith("/map_manage/"), `Unexpected floor image ${overlay.url}`);
    assert(overlay.r_x > overlay.l_x && overlay.r_y > overlay.l_y, `Invalid floor bounds ${id}`);
    floors.add(id);
    for (const pointId of floor.point_ids) {
      const region = pointById.get(pointId)?.area_id;
      if (region) regions.set(region, (regions.get(region) ?? 0) + 1);
    }
    return [{
      id, name: floor_name || floor_name_short, short: floor_name_short, url: overlay.url,
      bounds: [overlay.l_x + origin[0], overlay.l_y + origin[1], overlay.r_x + origin[0], overlay.r_y + origin[1]],
    }];
  });
  if (!levels.length) return [];
  const area = [...regions].sort((a, b) => b[1] - a[1])[0]?.[0] ?? 0;
  const defaultId = group.floors[group.default_floor_index]?.id;
  return [{ id: group.id, area, name: group.name || levels[0].name,
    defaultId: levels.some(level => level.id === defaultId) ? defaultId : levels[0].id, levels }];
});
for (const { id, label_id, x_pos: x, y_pos: y, point_group } of points) {
  const source = sources.get(label_id);
  assert(source, `Point without a map label: ${label_id}`);
  const px = Math.round(x + origin[0]), py = Math.round(y + origin[1]);
  assert(px >= 0 && py >= 0 && px < totalSize[0] && py < totalSize[1], `Point outside map ${mapId} bounds`);
  const floor = point_group?.floor_id;
  source.points.push([px, py, floors.has(floor) ? floor : 0, id]);
}
const available = [...sources.values()].filter(source => source.points.length);
assert(available.length > (mapId === 2 ? 500 : 10), `Incomplete map ${mapId} snapshot`);
const categories = labels.filter(label => available.some(source => source.category === label.id)).map(({ id, name }) => ({ id, name }));
const meta = { mapId, mapName, mapVersion, gameVersion, totalSize, minZoom, maxZoom };
writeFileSync(output, `{"meta":${JSON.stringify(meta)},"categories":${JSON.stringify(categories)},"floorGroups":${JSON.stringify(floorGroups)},"sources":[\n${available.map(source => JSON.stringify(source)).join(",\n")}\n]}\n`);
console.log(`${mapName}: ${available.length} types, ${points.length} markers, ${floors.size} floors`);
