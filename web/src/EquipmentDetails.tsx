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
      <p className="muted">
        Характеристики оружия отсутствуют в текущем справочнике.
      </p>
    );
  return (
    <section className="equipment-reference">
      <h3>Характеристики оружия</h3>
      <dl>
        {info.props.map((p: any) => (
          <div key={p.type}>
            <dt>{equipmentStatName(p.type)}</dt>
            <dd>{equipmentStatValue(p)}</dd>
          </div>
        ))}
      </dl>
      {info.props.length < 2 && (
        <p className="muted">Дополнительная характеристика отсутствует.</p>
      )}
      <h3>Эффект оружия</h3>
      {info.effect ? (
        <>
          <h4>
            {info.effect.name} · Пробуждение {info.refinement}
          </h4>
          <p className="equipment-description">{info.effect.description}</p>
        </>
      ) : (
        <p className="muted">
          {info.hasEffect
            ? "Эффект для текущего ранга пробуждения отсутствует в справочнике."
            : "У оружия нет эффекта пробуждения."}
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
      <h3>Бонус комплекта</h3>
      <h4>{set.name}</h4>
      {set.bonuses.map((b: any) => (
        <div key={b.pieces}>
          <strong>{b.pieces} предмета</strong>
          <p className="equipment-description">
            {b.description ?? "Описание отсутствует в текущем справочнике."}
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
      <span className="equipped-artifact-level">
        Ур. {values.level ?? "—"}{" "}
        <strong>
          {stats.main
            ? `${equipmentStatName(stats.main.type)} ${equipmentStatValue(stats.main)}`
            : "Основная характеристика не определена"}
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
        <small>Часть сабстатов не определена</small>
      )}
    </>
  );
}
