import { useEffect, useState, type ReactNode } from "react";
import { TooltipButton } from "./TooltipButton";

type Cycle = {
  starts_unix_s: number;
  ends_unix_s: number;
};

function CycleStat({ values, label, symbol, result, details }: {
  values?: Cycle;
  label: string;
  symbol: string;
  result: string;
  details?: ReactNode;
}) {
  const [now, setNow] = useState(Date.now);
  const ends = (values?.ends_unix_s ?? 0) * 1000 + 1000;
  useEffect(() => {
    let timer: number | undefined;
    const schedule = () => {
      const time = Date.now();
      setNow(time);
      if (time < ends) timer = window.setTimeout(schedule, Math.min(ends - time, 86_400_000));
    };
    schedule();
    return () => window.clearTimeout(timer);
  }, [ends]);

  const current = values && now >= values.starts_unix_s * 1000 && now < ends;
  return <TooltipButton className="header-stat abyss-stat" fallback={label}
    content={<p>{current
      ? <>{details && <>{details}<br /></>}
          Обновится {new Date(ends).toLocaleString("ru-RU")}.</>
      : "Ожидаем данные текущего цикла."}</p>}>
    <span className="header-stat-symbol" aria-hidden="true">{symbol}</span>
    <span><small>{label}</small><strong>{current ? result : "—"}</strong></span>
  </TooltipButton>;
}

export function AbyssStat({ values }: { values?: Cycle & {floor?: number; chamber?: number; stars: number} }) {
  const chamber = values?.floor ? `${values.floor}-${values.chamber}` : null;
  return <CycleStat values={values} label="Витая Бездна" symbol="✧"
    result={`${chamber ? chamber + " · " : ""}${values?.stars ?? 0}/36 ★`}
    details={chamber && `Этаж ${values?.floor}, зал ${values?.chamber}.`} />;
}

export function TheaterStat({ values }: { values?: Cycle & {acts: number; arcana: number; echo_keys?: number | null} }) {
  return <CycleStat values={values} label="Театр" symbol="◇"
    result={`${values?.acts ?? 0}/10 · ${values?.arcana ?? 0}/2`}
    details={values && <>
      Акты {values.acts}/10 · Арканы {values.arcana}/2.
      {values.echo_keys != null && <><br />Ключи эха за сезон {values.echo_keys}/2.</>}
    </>} />;
}

export function OnslaughtStat({ values }: { values?: Cycle & {difficulty: number; seconds: number} }) {
  const difficulty = ["I", "II", "III", "IV", "V", "VI"][(values?.difficulty ?? 0) - 1] ?? "—";
  return <CycleStat values={values} label="Натиск" symbol="⚔"
    result={difficulty}
    details={values && `Сложность ${difficulty}. Лучший рекорд — ${values.seconds} с.`} />;
}
