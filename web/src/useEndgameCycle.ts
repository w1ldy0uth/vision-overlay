import { useEffect, useState } from "react";
import { localizeGameData, uiLocale } from "./i18n.mjs";
import { serverOffsets as regionOffsets } from "./server-clock.mjs";

export type CycleSchedule = {
  schema_version: 1;
  schedule_id: number;
  starts_server_time: string;
  ends_server_time_inclusive: string;
};

export type CycleProgress = {
  schedule_id: number;
  starts_unix_s: number;
  ends_unix_s: number;
};

type Region = keyof typeof regionOffsets;
const dateTime = new Intl.DateTimeFormat(uiLocale, { dateStyle: "medium", timeStyle: "short" });

export function useEndgameCycle<C extends CycleSchedule, P extends CycleProgress>(
  mode: "abyss" | "theater" | "onslaught", profileRegion: string | undefined, progress: P | undefined,
) {
  const [cycle, setCycle] = useState<C | null>(null);
  const [loading, setLoading] = useState(true);
  const [manualRegion, setManualRegion] = useState<Region>("os_euro");
  const [now, setNow] = useState(Date.now);
  const hasProfileRegion = !!profileRegion && Object.hasOwn(regionOffsets, profileRegion);
  const region = hasProfileRegion ? profileRegion as Region : manualRegion;

  useEffect(() => {
    const request = new AbortController();
    void fetch(`/endgame/${mode}.json`, { signal: request.signal })
      .then(async response => {
        if (!response.ok) return;
        const payload: C = await response.json();
        if (!request.signal.aborted && payload.schema_version === 1) setCycle(localizeGameData(payload));
      })
      .catch(() => {})
      .finally(() => { if (!request.signal.aborted) setLoading(false); });
    return () => request.abort();
  }, [mode]);

  const toTimestamp = (value: string) => Date.parse(value.replace(" ", "T") + regionOffsets[region]);
  const starts = cycle ? toTimestamp(cycle.starts_server_time) : NaN;
  const ends = cycle ? toTimestamp(cycle.ends_server_time_inclusive) + 1000 : NaN;
  useEffect(() => {
    let timer: number | undefined;
    const schedule = () => {
      const time = Date.now();
      setNow(time);
      const nextBoundary = time < starts ? starts : ends;
      if (time < nextBoundary) timer = window.setTimeout(schedule, Math.min(nextBoundary - time, 86_400_000));
    };
    schedule();
    return () => window.clearTimeout(timer);
  }, [starts, ends]);

  const current = cycle && now >= starts && now < ends ? cycle : null;
  const result = current && hasProfileRegion && progress?.schedule_id === current.schedule_id
    && progress.starts_unix_s * 1000 === starts && (progress.ends_unix_s + 1) * 1000 === ends
    ? progress : undefined;
  return {
    current, result, hasProfileRegion, region, loading, starts, ends,
    endsLabel: current ? dateTime.format(ends) : "",
    setRegion: (value: string) => {
      if (Object.hasOwn(regionOffsets, value)) setManualRegion(value as Region);
    },
  };
}
