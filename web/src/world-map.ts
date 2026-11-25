export type MapPoint = [x: number, y: number, floorId: number, id: number];
export type Floor = { id: number; name: string; short: string; url: string; bounds: [number, number, number, number] };
export type FloorGroup = { id: number; area: number; name: string; defaultId: number; levels: Floor[] };
export type MapSource = {
  id: number;
  category: number;
  kind: "resource" | "enemy" | "boss" | "object";
  items: number[];
  name: string;
  icon: string;
  points: MapPoint[];
};
export type WorldMapData = {
  meta: {
    mapId: number;
    mapName: string;
    mapVersion: string;
    gameVersion: string;
    totalSize: [number, number];
    minZoom: number;
    maxZoom: number;
  };
  categories: { id: number; name: string }[];
  floorGroups: FloorGroup[];
  sources: MapSource[];
};
export type Camera = { x: number; y: number; zoom: number };
export type MapSize = { width: number; height: number };

export function worldMapTiles(meta: WorldMapData["meta"], camera: Camera, size: MapSize) {
  const scale = 2 ** camera.zoom;
  const z = Math.max(meta.minZoom, Math.min(meta.maxZoom, Math.floor(camera.zoom)));
  const cell = 256 / 2 ** z;
  const left = camera.x - size.width / (2 * scale);
  const top = camera.y - size.height / (2 * scale);
  const maxX = Math.min(Math.ceil(meta.totalSize[0] / cell) - 1, Math.floor((left + size.width / scale) / cell));
  const maxY = Math.min(Math.ceil(meta.totalSize[1] / cell) - 1, Math.floor((top + size.height / scale) / cell));
  const suffix = z < 0 ? `N${-z}` : `P${z}`;
  const base = `https://act-webstatic.hoyoverse.com/map_manage/map/${meta.mapId}/${meta.mapVersion}`;
  const tiles = [];
  for (let y = Math.max(0, Math.floor(top / cell)); y <= maxY; y++) {
    for (let x = Math.max(0, Math.floor(left / cell)); x <= maxX; x++) {
      const key = `${x}_${y}_${suffix}`;
      tiles.push({ key, url: `${base}/${key}.webp`, x: (x * cell - left) * scale, y: (y * cell - top) * scale, size: cell * scale });
    }
  }
  return tiles;
}

export function floorSources(sources: MapSource[], floorId: number | null) {
  if (floorId === null) return sources;
  return sources.flatMap(source => {
    const points = source.points.filter(point => point[2] === floorId);
    return points.length ? [{ ...source, points }] : [];
  });
}

type LocatedPoint = { source: MapSource; point: MapPoint };
const worldCell = 1024;

export function indexMapSources(sources: MapSource[]) {
  const buckets = new Map<string, LocatedPoint[]>();
  for (const source of sources) {
    for (const point of source.points) {
      const key = `${Math.floor(point[0] / worldCell)}:${Math.floor(point[1] / worldCell)}`;
      const located = { source, point };
      const bucket = buckets.get(key);
      if (bucket) bucket.push(located);
      else buckets.set(key, [located]);
    }
  }
  return buckets;
}

export function visibleMapPins(buckets: ReturnType<typeof indexMapSources>, camera: Camera, size: MapSize) {
  const scale = 2 ** camera.zoom;
  const width = size.width / scale, height = size.height / scale;
  const left = camera.x - width / 2, top = camera.y - height / 2;
  const cell = (camera.zoom < -1 ? 36 : 3) / scale;
  const pins = new Map<string, { key: string; x: number; y: number; points: LocatedPoint[] }>();
  for (let row = Math.floor(top / worldCell); row <= Math.floor((top + height) / worldCell); row++) {
    for (let col = Math.floor(left / worldCell); col <= Math.floor((left + width) / worldCell); col++) {
      for (const located of buckets.get(`${col}:${row}`) ?? []) {
        const [px, py] = located.point;
        const x = (px - left) * scale, y = (py - top) * scale;
        if (x < 0 || y < 0 || x > size.width || y > size.height) continue;
        const key = `${Math.floor(px / cell)}:${Math.floor(py / cell)}`;
        const pin = pins.get(key);
        if (pin) pin.points.push(located);
        else pins.set(key, { key, x, y, points: [located] });
      }
    }
  }
  return [...pins.values()];
}
