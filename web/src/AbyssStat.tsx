import { tr, uiLocale } from "./i18n.mjs";
import { useEffect, useState, type ReactNode } from "react";
import { TooltipButton } from "./TooltipButton";

type Cycle = {
  starts_unix_s: number;
  ends_unix_s: number;
};

function CycleStat({ values, label, symbol, result, details, href }: {
  values?: Cycle;
  label: string;
  symbol: string;
  result: string;
  details?: ReactNode;
  href?: string;
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
    actionHref={href} actionLabel={tr("Текущий цикл")}
    onClick={href ? () => { window.location.hash = href; } : undefined}
    content={<p>{current
      ? <>{details && <>{details}<br /></>}{tr("Обновится ")}{new Date(ends).toLocaleString(uiLocale)}.</>
      : tr("Ожидаем данные текущего цикла.")}</p>}>
    <span className="header-stat-symbol" aria-hidden="true">{symbol}</span>
    <span><small>{label}</small><strong>{current ? result : "—"}</strong></span>
  </TooltipButton>;
}

export function AbyssStat({ values }: { values?: Cycle & {floor?: number; chamber?: number; stars: number} }) {
  const chamber = values?.floor ? `${values.floor}-${values.chamber}` : null;
  return <CycleStat values={values} label={tr("Витая Бездна")} symbol="✧" href="#abyss"
    result={`${chamber ? chamber + " · " : ""}${values?.stars ?? 0}/36 ★`}
    details={chamber && tr("Этаж {0}, зал {1}.", values?.floor, values?.chamber)} />;
}

export function TheaterStat({ values }: { values?: Cycle & {acts: number; arcana?: number | null; echo_keys?: number | null} }) {
  const arcana = values?.arcana === 2 ? "2/2" : values?.arcana === 1 ? "≥1/2" : "—";
  return <CycleStat values={values} label={tr("Театр")} symbol="◇" href="#theater"
    result={`${values?.acts ?? 0}/10 · ${arcana}`}
    details={values && <>{tr("Акты ")}{values.acts}/10 · {values.arcana ? tr("Арканы {0}.", arcana) : tr("Данные об арканах неизвестны.")}
      {values.echo_keys != null && <><br />{tr("Ключи эха за сезон ")}{values.echo_keys}/2.</>}
    </>} />;
}

export const onslaughtDifficulties = ["I", "II", "III", "IV", "V", "VI"] as const;

export function OnslaughtStat({ values }: { values?: Cycle & {difficulty: number; seconds: number} }) {
  const difficulty = onslaughtDifficulties[(values?.difficulty ?? 0) - 1] ?? "—";
  return <CycleStat values={values} label={tr("Натиск")} symbol="⚔" href="#onslaught"
    result={difficulty}
    details={values && tr("Сложность {0}. Лучший рекорд — {1} с.", difficulty, values.seconds)} />;
}
