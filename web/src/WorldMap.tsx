import { tr } from "./i18n.mjs";
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { useMapExploration } from "./useMapExploration";
import { indexMapSources, visibleMapPins, worldMapTiles, type Camera, type Floor, type MapSize, type MapSource, type WorldMapData } from "./world-map";

type Pin = ReturnType<typeof visibleMapPins>[number];
const home: Camera = { x: 19500, y: 15300, zoom: -4 };
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));

export function WorldMap({ profileId, meta, sources, material, floor, needs, names }: {
  profileId?: string;
  meta: WorldMapData["meta"];
  sources: MapSource[];
  material: number;
  floor?: Floor;
  needs: Map<number, number | null> | null;
  names: Record<string, string>;
}) {
  const { explored, mark, saveError } = useMapExploration(profileId, meta.mapId);
  const viewport = useRef<HTMLDivElement>(null);
  const popup = useRef<HTMLElement>(null);
  const popupId = useId();
  const focusPopup = useRef(false);
  const drag = useRef<{ id: number; x: number; y: number; camera: Camera; pin?: Pin; moving: boolean } | null>(null);
  const focusedMaterial = useRef(0);
  const focusedFloor = useRef(0);
  const [size, setSize] = useState<MapSize>({ width: 0, height: 0 });
  const [camera, setCamera] = useState<Camera>(() => meta.mapId === 2 ? home : {
    x: meta.totalSize[0] / 2, y: meta.totalSize[1] / 2,
    zoom: Math.min(-2, Math.log2(420 / Math.max(...meta.totalSize))),
  });
  const [selection, setSelection] = useState<Pin["points"]>([]);
  const tiles = useMemo(() => worldMapTiles(meta, camera, size), [meta, camera, size]);
  const index = useMemo(() => indexMapSources(sources), [sources]);
  const pins = useMemo(() => visibleMapPins(index, camera, size), [index, camera, size]);
  const activeSelection = selection.filter(({ source }) => sources.includes(source));
  const anchor = activeSelection[0]?.point;
  const scale = 2 ** camera.zoom;
  const popupX = anchor ? (anchor[0] - camera.x) * scale + size.width / 2 : -1;
  const popupY = anchor ? (anchor[1] - camera.y) * scale + size.height / 2 : -1;
  const popupVisible = popupX >= 0 && popupX <= size.width && popupY >= 0 && popupY <= size.height;
  const above = popupY > size.height / 2;
  const popupWidth = Math.min(300, size.width - 16);
  const popupStyle = {
    left: clamp(popupX - popupWidth / 2, 8, size.width - popupWidth - 8),
    top: popupY + (above ? -18 : 18),
    width: popupWidth,
    maxHeight: (above ? popupY : size.height - popupY) - 26,
  };
  const image = floor && {
    left: (floor.bounds[0] - camera.x) * scale + size.width / 2,
    top: (floor.bounds[1] - camera.y) * scale + size.height / 2,
    width: (floor.bounds[2] - floor.bounds[0]) * scale,
    height: (floor.bounds[3] - floor.bounds[1]) * scale,
  };

  useEffect(() => {
    if (!popupVisible || !focusPopup.current) return;
    focusPopup.current = false;
    popup.current?.querySelector("input")?.focus({ preventScroll: true });
  }, [selection, popupVisible]);

  useEffect(() => {
    const floorId = floor?.id ?? 0;
    if (!size.width || !size.height || focusedFloor.current === floorId) return;
    focusedFloor.current = floorId;
    if (floor) fitBounds(floor.bounds);
  }, [floor, size]);

  useEffect(() => {
    if (!size.width || !size.height || focusedMaterial.current === material) return;
    focusedMaterial.current = material;
    if (material) centerSources();
  }, [material, size, sources]);

  useEffect(() => {
    const element = viewport.current;
    if (!element) return;
    const resize = new ResizeObserver(([entry]) => {
      setSize({ width: entry.contentRect.width, height: entry.contentRect.height });
    });
    resize.observe(element);
    function wheel(event: WheelEvent) {
      if ((event.target as HTMLElement).closest(".world-map-info")) return;
      event.preventDefault();
      const rect = element!.getBoundingClientRect();
      const x = event.clientX - rect.left - rect.width / 2;
      const y = event.clientY - rect.top - rect.height / 2;
      zoomBy(event.deltaY < 0 ? 0.5 : -0.5, x, y);
    }
    element.addEventListener("wheel", wheel, { passive: false });
    return () => { resize.disconnect(); element.removeEventListener("wheel", wheel); };
  }, [meta]);

  function contain(camera: Camera) {
    return { ...camera, x: clamp(camera.x, 0, meta.totalSize[0]), y: clamp(camera.y, 0, meta.totalSize[1]) };
  }

  function zoomBy(delta: number, x = 0, y = 0) {
    setCamera(current => {
      const zoom = clamp(current.zoom + delta, meta.minZoom - 2, meta.maxZoom + 1.5);
      const movement = 2 ** -current.zoom - 2 ** -zoom;
      return contain({ x: current.x + x * movement, y: current.y + y * movement, zoom });
    });
  }

  function centerSources() {
    const points = sources.flatMap(source => source.points);
    if (!points.length) return;
    const bounds = points.reduce(([left, top, right, bottom], [x, y]) =>
      [Math.min(left, x), Math.min(top, y), Math.max(right, x), Math.max(bottom, y)],
    [Infinity, Infinity, -Infinity, -Infinity]);
    fitBounds(bounds);
  }
  function fitBounds([left, top, right, bottom]: number[]) {
    const zoom = Math.log2(Math.min(size.width / (right - left + 400), size.height / (bottom - top + 400)));
    setCamera(contain({ x: (left + right) / 2, y: (top + bottom) / 2,
      zoom: clamp(zoom, meta.minZoom - 2, meta.maxZoom) }));
    setSelection([]);
  }

  function startDrag(event: PointerEvent<HTMLDivElement>) {
    if (!event.isPrimary || event.button !== 0 ||
      (event.target as HTMLElement).closest(".world-map-controls, .world-map-info")) return;
    event.preventDefault();
    const pinKey = (event.target as HTMLElement).closest<HTMLElement>(".world-map-pin")?.dataset.pin;
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, camera,
      pin: pinKey ? pins.find(pin => pin.key === pinKey) : undefined, moving: false };
  }
  function moveDrag(event: PointerEvent<HTMLDivElement>) {
    const start = drag.current;
    if (!start || start.id !== event.pointerId) return;
    if (!start.moving && Math.hypot(event.clientX - start.x, event.clientY - start.y) < 4) return;
    start.moving = true;
    const scale = 2 ** start.camera.zoom;
    setCamera(contain({ ...start.camera, x: start.camera.x - (event.clientX - start.x) / scale,
      y: start.camera.y - (event.clientY - start.y) / scale }));
  }
  function endDrag(event: PointerEvent<HTMLDivElement>) {
    const start = drag.current;
    if (!start || start.id !== event.pointerId) return;
    drag.current = null;
    if (start.moving) return;
    if (start.pin) selectPin(start.pin);
    else setSelection([]);
  }
  function keyboard(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape" && popupVisible) {
      event.preventDefault();
      closePopup();
      return;
    }
    if (event.target !== event.currentTarget) return;
    if (event.key === "+" || event.key === "-") {
      event.preventDefault();
      zoomBy(event.key === "+" ? 0.5 : -0.5);
    } else {
      const offset = { ArrowLeft: [-80, 0], ArrowRight: [80, 0], ArrowUp: [0, -80], ArrowDown: [0, 80] }[event.key];
      if (!offset) return;
      event.preventDefault();
      setCamera(current => contain({ ...current, x: current.x + offset[0] / 2 ** current.zoom,
        y: current.y + offset[1] / 2 ** current.zoom }));
    }
  }

  const name = (id: number) => names[id] ?? String(id);
  function selectPin(pin: Pin, keyboard = false) {
    if (pin.points.length > 1 && camera.zoom < -1) {
      const [x, y] = pin.points[0].point;
      setCamera({ x, y, zoom: Math.min(camera.zoom + 1, meta.maxZoom + 1.5) });
      setSelection([]);
    } else {
      focusPopup.current = keyboard;
      setSelection(pin.points);
    }
  }

  function closePopup() {
    if (popup.current?.contains(document.activeElement)) {
      const activePin = viewport.current?.querySelector<HTMLButtonElement>('[aria-controls="' + popupId + '"]');
      (activePin ?? viewport.current)?.focus({ preventScroll: true });
    }
    setSelection([]);
  }

  return (
    <div ref={viewport} className={`world-map-viewport${floor ? " world-map-floor" : ""}`} role="region" tabIndex={0} aria-label={meta.mapName}
      onPointerDown={startDrag} onPointerMove={moveDrag} onPointerUp={endDrag}
      onPointerCancel={() => { drag.current = null; }}
      onLostPointerCapture={() => { drag.current = null; }}
      onDragStart={event => event.preventDefault()}
      onKeyDown={keyboard}>
      {tiles.map(tile => <img key={tile.key} className="world-map-tile" src={tile.url}
        referrerPolicy="no-referrer" alt="" draggable={false}
        style={{ left: tile.x, top: tile.y, width: tile.size + 0.35, height: tile.size + 0.35 }} />)}
      {floor && <img className="world-map-floor-image" key={floor.id} src={floor.url} referrerPolicy="no-referrer"
        draggable={false} alt={floor.name} style={image} />}
      {pins.map(pin => {
        const unexplored = pin.points.find(({ point }) => !explored.has(point[3]));
        const { source } = unexplored ?? pin.points[0];
        const count = pin.points.length;
        const cluster = count > 1 && camera.zoom < -1;
        const title = count > 1 ? tr("{0} меток", count) : source.name;
        const label = unexplored ? title : tr("{0} · исследовано", title);
        const selected = popupVisible && pin.points.some(({ point }) => point[3] === anchor?.[3]);
        return <button type="button" key={pin.key} data-pin={pin.key}
          className={`world-map-pin ${cluster ? "cluster" : "object"}${unexplored ? "" : " explored"}`}
          style={{ left: pin.x, top: pin.y }} aria-label={label} title={label}
          aria-expanded={!cluster ? selected : undefined} aria-controls={selected ? popupId : undefined}
          onClick={event => { if (event.detail === 0) selectPin(pin, true); }}>{cluster ? count : <>
            <span className="world-map-pin-fallback" aria-hidden="true">◆</span>
            <img key={source.id} src={source.icon} referrerPolicy="no-referrer" alt="" draggable={false}
              onError={event => { event.currentTarget.hidden = true; }} />
          </>}</button>;
      })}
      <div className="world-map-controls">
        <button type="button" title={tr("Приблизить")} aria-label={tr("Приблизить")} onClick={() => zoomBy(.5)}>+</button>
        <button type="button" title={tr("Отдалить")} aria-label={tr("Отдалить")} onClick={() => zoomBy(-.5)}>−</button>
        <button type="button" title={tr("Показать все видимые по фильтрам метки")} aria-label={tr("Показать метки на карте")}
          disabled={!sources.length} onClick={centerSources}>⌖</button>
      </div>
      {popupVisible && <section ref={popup} id={popupId} className={`world-map-info${above ? " above" : ""}`}
        style={popupStyle} aria-label={tr("Выбранная метка")}>
        <button className="world-map-info-close" type="button" onClick={closePopup} aria-label={tr("Закрыть сведения о метке")}>×</button>
        {saveError && <p role="alert">{tr("Не удалось сохранить отметку в браузере.")}</p>}
        {activeSelection.map(({ source, point }, index) => {
          const id = point[3];
          const matching = material ? [material] : source.items.filter(id => !needs || needs.has(id));
          return <div key={id} className="world-map-point-details">
            <strong>{source.name}</strong>
            {floor && <span className="muted"> · {floor.name}</span>}
            <label className="world-map-exploration">
              <input type="checkbox" checked={explored.has(id)} onChange={event => mark(id, event.target.checked)}
                aria-label={activeSelection.length > 1 ? tr("Исследовано · {0} · метка {1}", source.name, index + 1) : undefined} />{tr("Исследовано")}</label>
            {matching.length > 0 && <ul>{matching.map(id => <li key={id}>
              {name(id)}{needs && (needs.get(id) == null ? tr(" · запас неизвестен") : tr(" · нужно ещё {0}", needs.get(id)))}
            </li>)}</ul>}
          </div>;
        })}
      </section>}
    </div>
  );
}
