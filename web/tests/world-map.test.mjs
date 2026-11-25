import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { floorSources, indexMapSources, visibleMapPins, worldMapTiles } from "../src/world-map.ts";

const { meta, categories, floorGroups, sources } = JSON.parse(readFileSync(new URL("../public/world-map.json", import.meta.url)));

test("world-map tiles and nearby markers use the same screen origin", () => {
  const camera = { x: 6144, y: 6144, zoom: -4 };
  const viewport = { width: 200, height: 200 };
  const first = worldMapTiles(meta, camera, viewport)[0];
  assert.equal(first.key, "1_1_N4");
  assert.equal(first.x, -28);
  assert.equal(first.y, -28);
  const source = { id: 2, kind: "resource", items: [100057], points: [[6144, 6144, 0, 1], [6160, 6176, 0, 2], [9000, 9000, 0, 3]] };
  const index = indexMapSources([source]);
  const pins = visibleMapPins(index, camera, viewport);
  assert.equal(pins.length, 1);
  assert.deepEqual([pins[0].x, pins[0].y], [100, 100]);
  assert.equal(pins[0].points.length, 2);
  assert.equal(visibleMapPins(index, { ...camera, zoom: -1 }, viewport).length, 2);
});

test("official point IDs uniquely identify markers even at identical coordinates", () => {
  const ids = new Set();
  let overlaps = 0;
  for (const source of sources) {
    const positions = new Set();
    for (const [x, y, floor, id] of source.points) {
      assert(Number.isSafeInteger(id) && id > 0);
      assert(!ids.has(id), `Duplicate point ID ${id}`);
      ids.add(id);
      const position = `${x}:${y}:${floor}`;
      if (positions.has(position)) overlaps++;
      positions.add(position);
    }
  }
  assert(overlaps > 0);
});

test("the full map includes official non-planner markers and icon URLs", () => {
  assert(categories.some(category => category.name === "Сокровища"));
  const waypoint = sources.find(source => source.id === 3);
  assert.equal(waypoint.name, "Точка телепортации");
  assert.deepEqual(waypoint.items, []);
  assert(waypoint.points.length > 0);
  assert(sources.every(source => source.icon.startsWith("https://")));
});

test("the snapshot maps all shaft rarities to the same farming markers", () => {
  const source = sources.find(source => source.items.includes(112122));
  assert.equal(source.kind, "enemy");
  assert.deepEqual(source.items, [112122, 112123, 112124]);
  assert(source.points.length > 0);
  for (const { points } of sources)
    assert(points.every(([x, y]) => x >= 0 && y >= 0 && x < meta.totalSize[0] && y < meta.totalSize[1]));
});

test("a Sumeru floor has its own overlay and only its assigned points", () => {
  const group = floorGroups.find(group => group.area === 4 && group.name.includes("гробниц") && group.levels.length > 1);
  const [first, second] = group.levels;
  assert.notEqual(first.url, second.url);
  const floor = floorSources(sources, first.id).flatMap(source => source.points);
  assert(floor.length > 0);
  assert(floor.every(point => point[2] === first.id));
  assert(floorSources(sources, 0).every(source => source.points.every(point => !point[2])));
});

test("Enkanomiya and Chasm use their own tile grids and marker sets", () => {
  for (const mapId of [7, 9]) {
    const map = JSON.parse(readFileSync(new URL(`../public/world-map-${mapId}.json`, import.meta.url)));
    assert.equal(map.meta.mapId, mapId);
    assert(map.sources.flatMap(source => source.points).length > 1000);
    const tiles = worldMapTiles(map.meta, { x: 2048, y: 2048, zoom: -3 }, { width: 500, height: 500 });
    assert(tiles[0].url.includes(`/map_manage/map/${mapId}/`));
  }
});
