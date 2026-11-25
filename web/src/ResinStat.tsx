import { tr } from "./i18n.mjs";
import { useEffect, useState } from "react";
import { TooltipButton } from "./TooltipButton";

const cap = 200;
const interval = 8 * 60;

function duration(seconds: number) {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor(seconds / 60) % 60;
  const remainder = seconds % 60;
  return hours ? tr("{0} ч {1} мин {2} с", hours, minutes, remainder) : tr("{0} мин {1} с", minutes, remainder);
}

export function ResinStat({ count, nextRecovery, live }: {
  count?: string;
  nextRecovery?: number | null;
  live: boolean;
}) {
  const [now, setNow] = useState(() => Date.now());
  const observed = count == null ? null : Number(count);
  useEffect(() => {
    if (!nextRecovery || observed === null || observed >= cap) return;
    const update = () => setNow(Date.now());
    update();
    const timer = window.setInterval(update, 1000);
    return () => window.clearInterval(timer);
  }, [nextRecovery, observed]);

  const remaining = nextRecovery ? nextRecovery - Math.floor(now / 1000) : null;
  const gained = remaining !== null && remaining <= 0 ? 1 + Math.floor(-remaining / interval) : 0;
  const current = observed === null ? null : Math.max(observed, Math.min(cap, observed + gained));
  const recovering = current !== null && current < cap && remaining !== null;
  const next = recovering ? remaining + gained * interval : null;
  const full = next !== null && current !== null ? next + (cap - current - 1) * interval : null;

  return <TooltipButton className="header-stat resin-stat" fallback={tr("Первородная смола")}
    content={<div className="resin-tooltip">
      {current === null ? <span>{tr("Ожидаем данные захвата.")}</span> :
        current >= cap ? <span>{tr("Первородная смола полностью восстановлена.")}</span> :
        next === null || full === null ? <span>{tr("Время восстановления пока недоступно.")}</span> :
        <>
          <div><span>{tr("До следующего восстановления")}</span><b>{duration(next)}</b></div>
          <div><span>{tr("До полного восстановления")}</span><b>{duration(full)}</b></div>
        </>}
      {current !== null && !live && <small>{nextRecovery ? tr("Расчёт по последним сохранённым данным.") : tr("Последнее сохранённое значение.")}</small>}
    </div>}>
    <span className="header-stat-symbol resin-symbol" aria-hidden="true">◔</span>
    <span><small>{tr("Первородная смола")}</small><strong>{current ?? "—"}/{cap}</strong></span>
  </TooltipButton>;
}
