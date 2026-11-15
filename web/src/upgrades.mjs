import { progressionReference } from "./progression.mjs";

const integer = (v) => {
  if (v === undefined || v === null || v === "") return null;
  const n = Number(v);
  return Number.isSafeInteger(n) && n >= 0 ? n : null;
};
const prop = (v, name, id) =>
  integer(v[name] ?? v.props?.[id]?.ival ?? v.props?.[id]?.val);

export function observedStock(data, id, wallet = false) {
  const rows = (
    data.collections?.[wallet ? "wallet" : "material_stacks"] ?? []
  ).filter((r) => Number(r.values.item_id) === Number(id));
  // Only an explicitly complete material inventory proves absent stacks are zero.
  // Partial/unverified snapshots and currencies retain the unknown semantics.
  if (!rows.length && !wallet) {
    const domain=data.domains?.find(d=>d.collection==='material_stacks');
    if (domain?.scope==='full' && ['synchronized','saved'].includes(domain.freshness)) return 0;
  }
  // Missing in an unverified snapshot is not a confirmed zero.
  if (
    !rows.length ||
    rows.some(
      (r) => r.confirmation !== "observed" || integer(r.values.count) === null,
    )
  )
    return null;
  const total = rows.reduce((sum, r) => sum + Number(r.values.count), 0);
  return Number.isSafeInteger(total) ? total : null;
}

/** Minimum supplied EXP, then minimum item count, bounded by observed stocks.
 * Binary splitting keeps complexity bounded even for inventories with 99999 ores.
 */
export function expMaterials(required, definitions, stock) {
  const items = Object.entries(definitions)
    .map(([id, exp]) => ({ id: Number(id), exp }))
    .sort((a, b) => b.exp - a.exp);
  if (!items.length || required <= 0) return null;
  const gcd = (a, b) => (b ? gcd(b, a % b) : a);
  const unit = items.reduce((n, item) => gcd(n, item.exp), 0);
  const limit =
    Math.ceil(required / unit) + Math.max(...items.map((i) => i.exp / unit));
  if (!Number.isSafeInteger(limit) || limit > 100000) return null;
  const solve = (bounded) => {
    const dp = Array(limit + 1).fill(null);
    dp[0] = { count: 0, materials: [] };
    for (const item of items) {
      const value = item.exp / unit;
      let remaining = Math.min(
        Math.floor(limit / value),
        bounded ? (stock(item.id) ?? 0) : Math.floor(limit / value),
      );
      for (let chunk = 1; remaining > 0; chunk *= 2) {
        const count = Math.min(chunk, remaining);
        remaining -= count;
        for (let sum = limit; sum >= value * count; sum--) {
          const previous = dp[sum - value * count];
          if (!previous || (dp[sum] && dp[sum].count <= previous.count + count))
            continue;
          dp[sum] = {
            count: previous.count + count,
            materials: [...previous.materials, { id: item.id, count }],
          };
        }
      }
    }
    for (let sum = Math.ceil(required / unit); sum <= limit; sum++) {
      if (!dp[sum]) continue;
      const merged = new Map();
      for (const m of dp[sum].materials)
        merged.set(m.id, (merged.get(m.id) ?? 0) + m.count);
      return {
        suppliedExp: sum * unit,
        materials: [...merged].map(([id, count]) => ({ id, count })),
        fromStock: bounded,
      };
    }
    return null;
  };
  return solve(true) ?? solve(false);
}

export function levelPreview(
  level,
  exp,
  cap,
  exps,
  materials,
  definitions,
  ratio,
  returnIds = [],
) {
  const suppliedExp = materials.reduce(
    (sum, m) => sum + definitions[String(m.id)] * m.count,
    0,
  );
  let resultLevel = level,
    resultExp = exp + suppliedExp;
  while (resultLevel < cap && resultExp >= exps[String(resultLevel)]) {
    resultExp -= exps[String(resultLevel)];
    resultLevel++;
  }
  const overflow = resultLevel === cap ? resultExp : 0;
  if (overflow) resultExp = 0;
  let lostExp = overflow;
  const returnedMaterials = [];
  for (const id of returnIds) {
    const count = Math.floor(lostExp / definitions[String(id)]);
    if (count) {
      returnedMaterials.push({ id, count });
      lostExp -= definitions[String(id)] * count;
    }
  }
  return {
    suppliedExp,
    resultLevel,
    resultExp,
    overflow,
    lostExp,
    returnedMaterials,
    mora: Math.ceil((suppliedExp * ratio.numerator) / ratio.denominator),
  };
}

function assess(step, data, extra = []) {
  const materials = (step.materials ?? []).map((m) => ({
    ...m,
    owned: observedStock(data, m.id),
  }));
  const moraOwned = observedStock(data, 202, true);
  const missing =
    materials.some((m) => m.owned !== null && m.owned < m.count) ||
    (moraOwned !== null && moraOwned < step.mora);
  const unknown =
    step.materialsAvailable === false ||
    materials.some((m) => m.owned === null) ||
    moraOwned === null;
  const blockers = extra.filter(Boolean);
  const collections = [
    "wallet",
    ...(materials.length ? ["material_stacks"] : []),
  ];
  const stale = collections.some(
    (name) =>
      data.domains?.find((d) => d.collection === name)?.freshness !==
      "synchronized",
  );
  return {
    ...step,
    materials,
    moraOwned,
    blockers,
    status: blockers.length
      ? "locked"
      : missing
        ? "shortage"
        : unknown
          ? "unknown"
          : stale
            ? "saved"
            : "ready",
  };
}

export function nextUpgrades(values, catalog, data) {
  const reference = progressionReference(values, catalog);
  if (!reference) return null;
  const character = reference.character;
  const level = character
    ? prop(values, "level", "4001")
    : integer(values.level);
  const exp = character ? prop(values, "exp", "1001") : integer(values.exp);
  const ascension = character
    ? prop(values, "ascension", "1002")
    : integer(values.ascension);
  const p = catalog.progression;
  const pathId = (character ? p.avatarPaths : p.weaponPaths)[
    String(character ? values.avatar_id : values.item_id)
  ];
  const path = (character ? p.avatarPromotions : p.weaponPromotions)[
    String(pathId)
  ];
  const current = path[String(ascension)];
  const rank = integer(data.collections?.adventure?.[0]?.values.rank);
  let progress;
  if (
    level === null ||
    ascension === null ||
    !current ||
    level > current.maxLevel ||
    level < Math.max(1, current.requiredLevel)
  ) {
    progress = {
      kind: "unknown",
      reason: "Уровень или этап возвышения не подтверждён.",
    };
  } else if (level === current.maxLevel) {
    const next = path[String(ascension + 1)];
    progress = next
      ? assess(
          { kind: "ascension", from: ascension, to: ascension + 1, ...next },
          data,
          [
            rank === null
              ? "Ранг приключений не получен."
              : rank < next.requiredAdventureRank
                ? `Нужен ранг приключений ${next.requiredAdventureRank}.`
                : null,
          ],
        )
      : { kind: "max", reason: "Достигнут максимальный уровень." };
  } else {
    const l = catalog.leveling;
    const rarity = integer(catalog.rarities[String(values.item_id)]);
    const exps = character ? l?.avatarExp : l?.weaponExp?.[String(rarity)];
    const ratio = character
      ? l?.avatarMoraPerExp
      : l?.weaponMoraPerExp?.[String(rarity)];
    const definitions = character ? l?.avatarMaterials : l?.weaponMaterials;
    const required = exps?.[String(level)];
    if (
      !required ||
      !ratio ||
      !definitions ||
      exp === null ||
      exp >= required
    ) {
      progress = {
        kind: "unknown",
        reason: "Для расчёта нужны подтверждённый опыт и таблица уровней.",
      };
    } else {
      const plan = expMaterials(required - exp, definitions, (id) =>
        observedStock(data, id),
      );
      if (!plan)
        progress = {
          kind: "unknown",
          reason: "Не удалось подобрать материалы опыта.",
        };
      else {
        const preview = levelPreview(
          level,
          exp,
          current.maxLevel,
          exps,
          plan.materials,
          definitions,
          ratio,
          character ? [] : l.weaponReturnMaterials,
        );
        progress = assess(
          {
            kind: "level",
            from: level,
            to: level + 1,
            currentExp: exp,
            requiredExp: required,
            neededExp: required - exp,
            ...plan,
            ...preview,
          },
          data,
        );
      }
    }
  }
  const talents = reference.talents.map((t) => {
    const base = integer(values.skill_levels?.[t.skillId]);
    const step = t.levels.find((s) => s.level === base + 1);
    if (base === null || base < 1 || base > 10)
      return {
        skillId: t.skillId,
        kind: "unknown",
        reason: "Базовый уровень таланта не получен.",
      };
    if (!step)
      return {
        skillId: t.skillId,
        kind: "max",
        reason: "Достигнут максимальный базовый уровень.",
      };
    return assess(
      { kind: "talent", skillId: t.skillId, from: base, to: base + 1, ...step },
      data,
      [
        step.requiredLevel === null
          ? "Требование к уровню не сопоставлено со справочником."
          : null,
        ascension === null
          ? "Этап возвышения не получен."
          : ascension < step.requiredAscension
            ? `Нужно возвышение ${step.requiredAscension} (ур. ${step.requiredLevel}).`
            : null,
        level === null
          ? "Уровень персонажа не получен."
          : step.requiredLevel !== null && level < step.requiredLevel
            ? `Нужен уровень ${step.requiredLevel}.`
            : null,
      ],
    );
  });
  const fresh =
    data.domains?.find(
      (d) => d.collection === (character ? "characters" : "weapons"),
    )?.freshness === "synchronized";
  for (const quote of [progress, ...talents])
    if (quote.status === "ready" && !fresh) quote.status = "saved";
  return { progress, talents };
}
