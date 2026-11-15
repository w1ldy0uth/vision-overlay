import { useEffect, useState } from "react";
import { get } from "./api";
const reasons: Record<string, string> = {
  pinned: "Защищено пользователем",
  active_capture: "Активный захват",
  recovery_or_current_state_dependency:
    "Нужно текущим данным или точке восстановления",
  open_delivery_checkpoint: "Доставка ещё не завершена",
  unfinished_or_error_capture: "Незавершённый или ошибочный захват",
  delivery_or_recording_not_finalized:
    "Журнал или запись ещё не закрыты / не доставлены",
  unregistered_diagnostics: "Старая диагностика требует отдельной проверки",
  last_three_automatic_backups: "Одна из трёх последних автоматических копий",
  manual_backup_inside: "В папке есть резервная копия",
  unverified_backup: "Копия отсутствует или не прошла проверку",
  missing: "Файл не найден после переноса",
};
function size(bytes: number) {
  return `${(bytes / 1024 / 1024).toLocaleString("ru-RU", { maximumFractionDigits: 1 })} МБ`;
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
        `Удалено объектов: ${result.removed}. Освобождено файлами: ${size(result.freed_bytes)}.`,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="account-panel">
      <h2>Хранение и очистка данных</h2>
      <p>
        Храним три завершённые пригодные точки восстановления на каждый профиль.
        Активный захват, недоставленные события и необходимые источники защищены
        отдельно.
      </p>
      <p>
        Автоматическая копия создаётся после завершения и полной доставки
        захвата, если данные изменились. Сохраняются три последние проверенные
        автоматические копии. Ручные копии и архивы отката не участвуют в этой
        ротации.
      </p>
      <button disabled={busy} onClick={refresh}>
        Пересчитать план очистки
      </button>
      {busy && <p role="status">Проверяем состояние и зависимости…</p>}
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
          <h2>Точки восстановления · {plan.points.length}</h2>
          {!plan.points.length && (
            <p className="footnote">
              Завершённых точек с полным набором подтверждённых разделов пока
              нет. Неполные входы не вытесняют пригодные точки.
            </p>
          )}
          <div className="retention-points">
            {plan.points.map((p: any) => (
              <div key={p.point_id}>
                <b>
                  {p.uid} · {p.server_region}
                </b>
                <small>
                  {new Date(Number(p.observed_ms)).toLocaleString("ru-RU")}
                </small>
              </div>
            ))}
          </div>
          <h2>Файлы захвата и диагностики</h2>
          <p className="footnote">
            Ранее существовавшие папки изначально защищены. Снятие защиты
            разрешает их участие в следующем расчёте; другие причины защиты
            остаются в силе. Неизвестная старая диагностика автоматически не
            удаляется.
          </p>
          <div className="retention-files">
            {plan.artifacts.map((a: any) => (
              <article key={a.artifact_id}>
                <div>
                  <b>{a.path}</b>
                  <small>
                    {size(a.bytes)} ·{" "}
                    {a.kind === "automatic_backup"
                      ? "Автоматическая копия"
                      : a.kind === "capture"
                        ? "Захват"
                        : "Диагностика"}
                  </small>
                  <p>
                    {a.eligible
                      ? "Можно очистить"
                      : a.protected_reasons
                          .map(
                            (r: string) =>
                              reasons[r] ??
                              (r.startsWith("missing_or_unsafe")
                                ? "Недоступный или небезопасный путь"
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
                  />
                  Защитить
                </label>
              </article>
            ))}
          </div>
          <p>
            Доступно к очистке: <b>{plan.eligible_count}</b> объектов ·{" "}
            <b>{size(plan.eligible_bytes)}</b>.
          </p>
          <label>
            <input
              type="checkbox"
              checked={confirm}
              disabled={busy || !plan.eligible_count}
              onChange={(e) => setConfirm(e.target.checked)}
            />{" "}
            Удалить перечисленные доступные файлы и их устаревшие
            диагностические записи
          </label>
          <p>
            <button
              disabled={busy || !confirm || !plan.eligible_count}
              onClick={apply}
            >
              Очистить доступные данные
            </button>
          </p>
          <p className="footnote">
            Перед очисткой зависимости проверяются повторно. Освобождение
            страниц внутри SQLite не обязательно сразу уменьшает размер её
            файла; эти страницы используются последующими записями.
          </p>
        </>
      )}
    </section>
  );
}
