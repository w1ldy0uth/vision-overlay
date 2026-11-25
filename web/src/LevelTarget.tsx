import { tr } from "./i18n.mjs";
import { levelTargets, targetIndex } from "./goal-targets.mjs";

type Target = { level: number; ascension: number };
const checkpoints = [20, 40, 50, 60, 70, 80, 90];

function label(target: Target) {
  const plus = target.ascension === checkpoints.indexOf(target.level) + 1 && checkpoints.includes(target.level) && target.level < 90;
  return `${target.level}${plus ? "+" : ""}`;
}

export function LevelTarget({ value, max = 90, onChange }: {
  value: Target;
  max?: number;
  onChange: (target: Target) => void;
}) {
  const targets: Target[] = levelTargets(max);
  const index = targetIndex(targets, value);
  const current = targets[index];
  const select = (target: Target) => onChange({level: target.level, ascension: target.ascension});

  return <div className="goal-level-control">
    <details className="goal-level-options">
      <summary>{tr("Уровень ")}<strong>{label(current)}</strong></summary>
      <div className="goal-level-grid">
        {checkpoints.filter(level => level <= max).map(level => {
          const atLevel = targets.filter(target => target.level === level);
          return <div className="goal-level-row" key={level}>
            {atLevel.map(target => <button key={target.ascension} type="button"
              aria-pressed={target === current}
              onClick={event => { select(target); event.currentTarget.closest("details")?.removeAttribute("open"); }}>
              {label(target)}
            </button>)}
          </div>;
        })}
      </div>
    </details>
    <div className="goal-level-step">
      <button type="button" aria-label={tr("Повысить целевой уровень")} disabled={index >= targets.length - 1} onClick={() => select(targets[index + 1])}>+</button>
      <button type="button" aria-label={tr("Понизить целевой уровень")} disabled={index <= 0} onClick={() => select(targets[index - 1])}>−</button>
    </div>
    <span className="footnote">{tr("Возвышение ")}{current.ascension}</span>
  </div>;
}
