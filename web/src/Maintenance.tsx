import { tr, uiLocale } from "./i18n.mjs";
import { useEffect, useState } from "react";
import { get } from "./api";
const reasons: Record<string, string> = {
  pinned: tr("Защищено пользователем"),
  active_capture: tr("Активный захват"),
  recovery_or_current_state_dependency:
    tr("Нужно текущим данным или точке восстановления"),
  open_delivery_checkpoint: tr("Доставка ещё не завершена"),
  unfinished_or_error_capture: tr("Незавершённый или ошибочный захват"),
  delivery_or_recording_not_finalized:
    tr("Журнал или запись ещё не закрыты / не доставлены"),
  unregistered_diagnostics: tr("Старая диагностика требует отдельной проверки"),
  last_three_automatic_backups: tr("Одна из трёх последних автоматических копий"),
  manual_backup_inside: tr("В папке есть резервная копия"),
  unverified_backup: tr("Копия отсутствует или не прошла проверку"),
  missing: tr("Файл не найден после переноса"),
};
function size(bytes: number) {
  return tr("{0} МБ", (bytes / 1024 / 1024).toLocaleString(uiLocale, { maximumFractionDigits: 1 }));
}
export function Maintenance() {
  const [plan, setPlan] = useState<any>(null),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [message, setMessage] = useState(""),
    [confirm, setConfirm] = useState(false);
  async function refresh() {
    setBusy(true);
    setError("");
    setConfirm(false);
    try {
      setPlan(await get("/api/v1/maintenance"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    refresh();
  }, []);
  async function post(path: string, body: any) {
    const response = await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Vision-Control": "1" },
      body: JSON.stringify(body),
    });
    const value = await response.json();
    if (!response.ok)
      throw new Error(value.error?.message ?? `HTTP ${response.status}`);
    return value;
  }
  async function pin(id: string, pinned: boolean) {
    setBusy(true);
    setError("");
    setConfirm(false);
    try {
      setPlan(
        await post("/api/v1/maintenance/pin", { artifact_id: id, pinned }),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function apply() {
    if (!plan || !confirm) return;
    setBusy(true);
    setError("");
    setMessage("");
    setConfirm(false);
    try {
      const result = await post("/api/v1/maintenance/apply", {
        plan_id: plan.plan_id,
      });
      setPlan(result.preview);
      setMessage(
        tr("Удалено объектов: {0}. Освобождено файлами: {1}.", result.removed, size(result.freed_bytes)),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="account-panel">
      <h2>{tr("Хранение и очистка данных")}</h2>
      <p>{tr("Храним три завершённые пригодные точки восстановления на каждый профиль. Активный захват, недоставленные события и необходимые источники защищены отдельно.")}</p>
      <p>{tr("Автоматическая копия создаётся после завершения и полной доставки захвата, если данные изменились. Сохраняются три последние проверенные автоматические копии. Ручные копии и архивы отката не участвуют в этой ротации.")}</p>
      <button disabled={busy} onClick={refresh}>{tr("Пересчитать план очистки")}</button>
      {busy && <p role="status">{tr("Проверяем состояние и зависимости…")}</p>}
      {error && (
        <p className="banner error" role="alert">
          {error}
        </p>
      )}
      {message && (
        <p className="banner" role="status">
          {message}
        </p>
      )}
      {plan && (
        <>
          <h2>{tr("Точки восстановления · ")}{plan.points.length}</h2>
          {!plan.points.length && (
            <p className="footnote">{tr("Завершённых точек с полным набором подтверждённых разделов пока нет. Неполные входы не вытесняют пригодные точки.")}</p>
          )}
          <div className="retention-points">
            {plan.points.map((p: any) => (
              <div key={p.point_id}>
                <b>
                  {p.uid} · {p.server_region}
                </b>
                <small>
                  {new Date(Number(p.observed_ms)).toLocaleString(uiLocale)}
                </small>
              </div>
            ))}
          </div>
          <h2>{tr("Файлы захвата и диагностики")}</h2>
          <p className="footnote">{tr("Ранее существовавшие папки изначально защищены. Снятие защиты разрешает их участие в следующем расчёте; другие причины защиты остаются в силе. Неизвестная старая диагностика автоматически не удаляется.")}</p>
          <div className="retention-files">
            {plan.artifacts.map((a: any) => (
              <article key={a.artifact_id}>
                <div>
                  <b>{a.path}</b>
                  <small>
                    {size(a.bytes)} ·{" "}
                    {a.kind === "automatic_backup"
                      ? tr("Автоматическая копия")
                      : a.kind === "capture"
                        ? tr("Захват")
                        : tr("Диагностика")}
                  </small>
                  <p>
                    {a.eligible
                      ? tr("Можно очистить")
                      : a.protected_reasons
                          .map(
                            (r: string) =>
                              reasons[r] ??
                              (r.startsWith("missing_or_unsafe")
                                ? tr("Недоступный или небезопасный путь")
                                : r),
                          )
                          .join(" · ")}
                  </p>
                </div>
                <label>
                  <input
                    type="checkbox"
                    checked={a.pinned}
                    disabled={busy}
                    onChange={(e) => pin(a.artifact_id, e.target.checked)}
                  />{tr("Защитить")}</label>
              </article>
            ))}
          </div>
          <p>{tr("Доступно к очистке: ")}<b>{plan.eligible_count}</b>{tr(" объектов ·")}{" "}
            <b>{size(plan.eligible_bytes)}</b>.
          </p>
          <label>
            <input
              type="checkbox"
              checked={confirm}
              disabled={busy || !plan.eligible_count}
              onChange={(e) => setConfirm(e.target.checked)}
            />{" "}{tr("Удалить перечисленные доступные файлы и их устаревшие диагностические записи")}</label>
          <p>
            <button
              disabled={busy || !confirm || !plan.eligible_count}
              onClick={apply}
            >{tr("Очистить доступные данные")}</button>
          </p>
          <p className="footnote">{tr("Перед очисткой зависимости проверяются повторно. Освобождение страниц внутри SQLite не обязательно сразу уменьшает размер её файла; эти страницы используются последующими записями.")}</p>
        </>
      )}
    </section>
  );
}
