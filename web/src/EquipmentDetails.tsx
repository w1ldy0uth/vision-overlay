import { tr } from "./i18n.mjs";
import {
  weaponDetails,
  artifactStats,
  artifactSet,
  equipmentStatName,
  equipmentStatValue,
} from "./equipment-details.mjs";
export function WeaponDetails({ values, catalog }: any) {
  const info = weaponDetails(values, catalog);
  if (!info)
    return (
      <p className="muted">{tr("Характеристики оружия отсутствуют в текущем справочнике.")}</p>
    );
  return (
    <section className="equipment-reference">
      <h3>{tr("Характеристики оружия")}</h3>
      <dl>
        {info.props.map((p: any) => (
          <div key={p.type}>
            <dt>{equipmentStatName(p.type)}</dt>
            <dd>{equipmentStatValue(p)}</dd>
          </div>
        ))}
      </dl>
      {info.props.length < 2 && (
        <p className="muted">{tr("Дополнительная характеристика отсутствует.")}</p>
      )}
      <h3>{tr("Эффект оружия")}</h3>
      {info.effect ? (
        <>
          <h4>
            {info.effect.name}{tr(" · Пробуждение ")}{info.refinement}
          </h4>
          <p className="equipment-description">{info.effect.description}</p>
        </>
      ) : (
        <p className="muted">
          {info.hasEffect
            ? tr("Эффект для текущего ранга пробуждения отсутствует в справочнике.")
            : tr("У оружия нет эффекта пробуждения.")}
        </p>
      )}
    </section>
  );
}
export function ArtifactSetDetails({ values, catalog }: any) {
  const set = artifactSet(values, catalog);
  if (!set) return null;
  return (
    <section className="equipment-reference">
      <h3>{tr("Бонус комплекта")}</h3>
      <h4>{set.name}</h4>
      {set.bonuses.map((b: any) => (
        <div key={b.pieces}>
          <strong>{b.pieces}{tr(" предмета")}</strong>
          <p className="equipment-description">
            {b.description ?? tr("Описание отсутствует в текущем справочнике.")}
          </p>
        </div>
      ))}
    </section>
  );
}

export function ArtifactStatsInline({ values, catalog }: any) {
  const stats = artifactStats(values, catalog);
  return (
    <>
      <span className="equipped-artifact-level">{tr("Ур. ")}{values.level ?? "—"}{" "}
        <strong>
          {stats.main
            ? `${equipmentStatName(stats.main.type)} ${equipmentStatValue(stats.main)}`
            : tr("Основная характеристика не определена")}
        </strong>
      </span>
      <span className="equipped-artifact-substats">
        {stats.substats.map((p: any) => (
          <span key={p.type}>
            {equipmentStatName(p.type)} {equipmentStatValue(p)}
          </span>
        ))}
      </span>
      {stats.unknownSubstats > 0 && (
        <small>{tr("Часть сабстатов не определена")}</small>
      )}
    </>
  );
}
