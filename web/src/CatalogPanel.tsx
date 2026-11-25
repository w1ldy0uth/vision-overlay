import { tr, uiLocale } from "./i18n.mjs";
import { useEffect, useRef, useState } from "react";
import { get } from "./api";
const controls = {
  "Content-Type": "application/json",
  "X-Vision-Control": "1",
};
const short = (s?: string) => s?.slice(0, 12) ?? "—";
async function action(path: string, body: unknown, raw = false) {
  const response = await fetch(`/api/v1/catalog/${path}`, {
    method: "POST",
    headers: controls,
    body: raw ? (body as string) : JSON.stringify(body),
  });
  const data = await response.json();
  if (!response.ok)
    throw new Error(data.error?.message ?? `HTTP ${response.status}`);
  return data;
}
export function CatalogPanel({ onChanged }: { onChanged: () => void }) {
  const [status, setStatus] = useState<any>(null),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [connectionError, setConnectionError] = useState(""),
    [message, setMessage] = useState(""),
    [revision, setRevision] = useState("");
  const file = useRef<HTMLInputElement>(null);
  useEffect(() => {
    let disposed = false;
    const refresh = () =>
      get("/api/v1/catalog")
        .then((s) => {
          if (!disposed) {
            setStatus(s);
            setConnectionError("");
          }
        })
        .catch((e) => {
          if (!disposed) setConnectionError(e.message);
        });
    void refresh();
    const timer = setInterval(refresh, 5000);
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, []);
  async function run(path: string, body: unknown, notice: string, raw = false) {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const next = await action(path, body, raw);
      setStatus(next);
      setMessage(notice);
      if (["activate", "rollback"].includes(path)) onChanged();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  const current = status?.current,
    candidate = status?.candidate;
  const blocked = busy || status?.busy;
  const version = (v: any) => (
    <>
      <strong title={v?.manifest.revision}>{tr("Ревизия ")}{short(v?.manifest.revision)}
      </strong>
      <p className="muted">
        {v?.counts.items?.toLocaleString(uiLocale)}{tr(" предметов ·")}{" "}
        {v?.counts.characters}{tr(" записей персонажей ·")}{" "}
        {v?.counts.achievements_playable != null
          ? tr("{0} игровых достижений · {1} неиспользуемых/исключённых записей", v.counts.achievements_playable, v.counts.achievements_excluded)
          : tr("{0} определений достижений (включая неиспользуемые)", v?.counts.achievements)}
      </p>
      {v?.counts.avatar_promotion_paths != null && (
        <p className="muted">{tr("Стоимости: ")}{v.counts.avatar_promotion_paths}{tr(" путей возвышения персонажей · ")}{v.counts.weapon_promotion_paths}{tr(" путей оружия ·")}{" "}
          {v.counts.talent_cost_groups}{tr(" схем талантов")}</p>
      )}
      <small className="muted">
        {v?.manifest.language?.toUpperCase()}{tr(" · формат")}{" "}
        {v?.manifest.format_version} ·{" "}
        {v?.manifest.source === "bundled"
          ? tr("Встроенная версия")
          : "Dimbreath / AnimeGameData"}
      </small>
    </>
  );
  return (
    <section className="panel catalog-panel">
      <h2>{tr("Справочник игровых данных")}</h2>
      <p className="muted">{tr("Названия, иконки, категории и характеристики обновляются отдельно от приложения. Сначала новая версия скачивается и проверяется, затем её можно применить.")}</p>
      {current && (
        <div className="catalog-version">
          <span className="eyebrow">{tr("ТЕКУЩАЯ ВЕРСИЯ")}</span>
          {version(current)}
        </div>
      )}
      <div className="catalog-actions">
        <button
          disabled={blocked || !current}
          onClick={() =>
            run("prepare", {}, tr("Новая версия проверена и готова к просмотру."))
          }
        >{tr("Проверить обновления")}</button>
        <button
          disabled={blocked || !status?.previous}
          onClick={() =>
            run(
              "rollback",
              { expected_active: current.id },
              tr("Предыдущая рабочая версия восстановлена."),
            )
          }
        >{tr("Вернуть предыдущую версию")}</button>
        <a href="/api/v1/catalog/export" download="VisionOverlay.visioncatalog">{tr("Сохранить текущую версию")}</a>
        <button disabled={blocked} onClick={() => file.current?.click()}>{tr("Загрузить из файла")}</button>
        <input
          ref={file}
          type="file"
          accept=".visioncatalog,application/json"
          hidden
          onChange={async (e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (f) {
              if (f.size > 32 * 1024 * 1024) {
                setError(tr("Файл справочника превышает 32 МиБ."));
                return;
              }
              await run(
                "import",
                await f.text(),
                tr("Файл проверен и готов к просмотру."),
                true,
              );
            }
          }}
        />
      </div>
      <details className="catalog-advanced">
        <summary>{tr("Выбрать конкретную ревизию")}</summary>
        <p className="muted">{tr("Для воспроизводимой проверки можно указать полный идентификатор ревизии источника.")}</p>
        <div className="catalog-actions">
          <input
            aria-label={tr("Ревизия справочника")}
            value={revision}
            onChange={(e) => setRevision(e.target.value.trim())}
            placeholder={tr("40 символов: a–f, 0–9")}
            maxLength={40}
          />
          <button
            disabled={blocked || !/^[a-f0-9]{40}$/.test(revision)}
            onClick={() =>
              run("prepare", { revision }, tr("Выбранная ревизия проверена."))
            }
          >{tr("Подготовить ревизию")}</button>
        </div>
      </details>
      {blocked && (
        <p role="status">{tr("Обработка справочника… Текущая версия продолжает работать.")}</p>
      )}
      {connectionError && (
        <div className="banner warn" role="status">{tr("Не удалось обновить состояние справочника. Повторяем подключение…")}</div>
      )}
      {error && (
        <div className="banner error" role="alert">{tr("Обновление не применено. ")}{error}
        </div>
      )}
      {!error && status?.last_error && (
        <div className="banner" role="alert">
          {status.last_error}
        </div>
      )}
      {message && <p role="status">{message}</p>}
      {candidate && (
        <div className="catalog-candidate">
          <span className="eyebrow">{tr("ПОДГОТОВЛЕННАЯ ВЕРСИЯ")}</span>
          {version(candidate.version)}
          <dl className="catalog-changes">
            {Object.entries({
              names: tr("Названия предметов"),
              items: tr("Карточки предметов"),
              materials: tr("Категории материалов"),
              avatarNames: tr("Имена персонажей"),
              skillNames: tr("Названия навыков"),
              achievements: tr("Достижения"),
            }).map(([key, label]) => {
              const delta = candidate.changes?.[key];
              return delta &&
                (delta.added || delta.removed || delta.changed) ? (
                <div key={key}>
                  <dt>{label}</dt>
                  <dd>{tr("Добавлено: ")}{delta.added}{tr(" · изменено: ")}{delta.changed}{tr(" · удалено: ")}{delta.removed}
                  </dd>
                </div>
              ) : null;
            })}
            {candidate.changes?.stat_tables_changed && (
              <div>
                <dt>{tr("Характеристики артефактов")}</dt>
                <dd>{tr("Таблицы обновлены")}</dd>
              </div>
            )}
          </dl>
          {candidate.changes?.talent_links_changed && (
            <p className="muted">{tr("Обновлены связи навыков, созвездий и типы персонажей.")}</p>
          )}
          {candidate.changes?.progression_changed && (
            <p className="muted">{tr("Обновлены материалы, мора и требования улучшений.")}</p>
          )}
          {candidate.changes?.profile_presentation_changed && (
            <p className="muted">{tr("Обновлены аватарки профиля и пороги опыта приключений.")}</p>
          )}
          {candidate.changes?.crafting_changed && (
            <p className="muted">{tr("Обновлены гарантированные рецепты крафта, стоимость и требования ранга.")}</p>
          )}
          {candidate.changes?.leveling_changed && (
            <p className="muted">{tr("Обновлены таблицы опыта, цена повышения уровней и материалы опыта.")}</p>
          )}
          <p>{tr("Проверены контрольные суммы, формат и связи справочника. Данные аккаунтов останутся на месте.")}</p>
          {candidate.unchanged ? (
            <p className="muted">{tr("Содержимое совпадает с текущей версией.")}</p>
          ) : (
            <button
              disabled={blocked || candidate.base_active !== current?.id}
              onClick={() =>
                run(
                  "activate",
                  { id: candidate.version.id, expected_active: current.id },
                  tr("Справочник применён. Открытые вкладки подхватят его автоматически."),
                )
              }
            >{tr("Применить подготовленную версию")}</button>
          )}
          {candidate.base_active !== current?.id && (
            <p className="muted">{tr("Рабочая версия уже изменилась. Подготовьте обновление заново.")}</p>
          )}
        </div>
      )}
      {status?.previous && (
        <p className="footnote">{tr("Для отката доступна ревизия ")}{short(status.previous.manifest.revision)}{tr(". Приложение использует последнюю рабочую версию при ошибке обновления.")}</p>
      )}
    </section>
  );
}
