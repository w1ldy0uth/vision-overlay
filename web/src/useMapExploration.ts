import { useEffect, useState } from "react";

function readPoints(key: string): Set<number> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) ?? "[]");
    return new Set(Array.isArray(value) ? value.filter(id => Number.isSafeInteger(id) && id > 0) : []);
  } catch {
    return new Set();
  }
}

export function useMapExploration(profileId: string | undefined, mapId: number) {
  const key = `vo.map.explored.${profileId ?? "local"}.${mapId}`;
  const [explored, setExplored] = useState(() => readPoints(key));
  const [saveError, setSaveError] = useState(false);

  useEffect(() => {
    const sync = (event: StorageEvent) => {
      if (event.key === key || event.key === null) setExplored(readPoints(key));
    };
    window.addEventListener("storage", sync);
    return () => window.removeEventListener("storage", sync);
  }, [key]);

  function mark(id: number, checked: boolean) {
    const next = readPoints(key);
    if (checked) next.add(id);
    else next.delete(id);
    try {
      if (next.size) localStorage.setItem(key, JSON.stringify([...next]));
      else localStorage.removeItem(key);
      setExplored(next);
      setSaveError(false);
    } catch {
      setSaveError(true);
    }
  }

  return { explored, mark, saveError };
}
