import { useState, useRef } from "react";
import { get } from "./api";

async function response(response: Response) {
  if (!response.ok) {
    const value = await response.json();
    throw new Error(value.error?.message ?? `HTTP ${response.status}`);
  }
  return response;
}
const headers = { "X-Vision-Control": "1" };
export function Backups({
  selected,
  catalog,
}: {
  selected: string;
  catalog: any;
}) {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [message, setMessage] = useState(""),
    [preview, setPreview] = useState<any>(null),
    [confirmed, setConfirmed] = useState(false);
  const file = useRef<File | null>(null);
  async function save() {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const filename = `VisionOverlay-${new Date().toISOString().replaceAll(":", "-")}.visionbackup`;
      const picker = (window as any).showSaveFilePicker;
      const destination = picker
        ? await picker({
            suggestedName: filename,
            types: [
              {
                description: "Резервная копия Vision Overlay",
                accept: { "application/octet-stream": [".visionbackup"] },
              },
            ],
          })
        : null;
      const r = await response(
        await fetch("/api/v1/backups/export", {
          method: "POST",
          headers: { ...headers, "Content-Type": "application/json" },
          body: JSON.stringify({
            settings: {
              schema_version: 1,
              locale: "ru",
              selected_profile_id: selected || null,
            },
            catalog: { schema_version: 1, sources: catalog.revisions ?? {} },
          }),
        }),
      );
      const blob = await r.blob();
      if (destination) {
        const writer = await destination.createWritable();
        await writer.write(blob);
        await writer.close();
        setMessage("Резервная копия сохранена в выбранный файл.");
      } else {
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = filename;
        link.click();
        setTimeout(() => URL.revokeObjectURL(url), 60000);
        setMessage(
          "Копия передана браузеру для скачивания. Путь сохранения определяется настройками браузера.",
        );
      }
    } catch (e) {
      if ((e as Error).name !== "AbortError") setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function inspect(selectedFile: File | null) {
    file.current = selectedFile;
    setPreview(null);
    setConfirmed(false);
    setError("");
    setMessage("");
    if (!selectedFile) return;
    setBusy(true);
    try {
      if (selectedFile.size > 512 * 1024 * 1024)
        throw new Error("Архив больше 512 МБ");
      const r = await response(
        await fetch("/api/v1/backups/inspect", {
          method: "POST",
          headers,
          body: selectedFile,
        }),
      );
      setPreview(await r.json());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function restore() {
    if (!file.current || !preview || !confirmed) return;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const automation = await get("/api/v1/capture/automation");
      if (automation.capture_run_id || automation.external_capture)
        throw new Error(
          "Остановите захват перед восстановлением. После восстановления потребуется новый вход в игру.",
        );
      const r = await response(
        await fetch("/api/v1/backups/restore", {
          method: "POST",
          headers: {
            ...headers,
            "X-Vision-Restore": "replace-all",
            "X-Backup-Sha256": preview.sha256,
          },
          body: file.current,
        }),
      );
      const result = await r.json();
      if (result.settings?.selected_profile_id)
        localStorage.setItem("vo.profile", result.settings.selected_profile_id);
      else localStorage.removeItem("vo.profile");
      setMessage(
        `Данные восстановлены. Копия для отката: ${result.rollback_path}. ${result.region_settings_warning ?? ""}`,
      );
      setPreview(null);
      setConfirmed(false);
      file.current = null;
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="account-panel">
      <h2>Резервная копия всех профилей</h2>
      <p>
        Содержит сохранённые данные, историю и переносимые настройки. Ключи
        декодера, токен API и сетевые записи в архив не входят.
      </p>
      <p className="footnote">
        Копия фиксирует уже записанное в БД состояние. При отставании доставки
        последние события захвата могут ещё не попасть в неё. Автоматическое
        создание и ротация пока не настроены.
      </p>
      <button disabled={busy} onClick={save}>
        Выбрать файл и сохранить копию
      </button>
      <hr />
      <h2>Восстановить из копии</h2>
      <p>
        Восстановление заменит все локальные профили. Сначала закройте игру и
        дождитесь остановки захвата. Текущие данные будут сохранены отдельным
        архивом для отката.
      </p>
      <input
        type="file"
        accept=".visionbackup"
        aria-label="Файл резервной копии"
        disabled={busy}
        onChange={(e) => inspect(e.target.files?.[0] ?? null)}
      />
      {preview && (
        <div className="backup-preview">
          <p>
            Проверена копия от{" "}
            {new Date(preview.manifest.created_at).toLocaleString("ru-RU")} ·
            приложение {preview.manifest.application_version} · ревизия{" "}
            {preview.manifest.revision}
          </p>
          <ul>
            {preview.manifest.profiles.map((p: any) => (
              <li key={p.profile_id}>
                {p.uid} · {p.server_region}
              </li>
            ))}
          </ul>
          <p className="footnote">
            Автозапуск и привилегированный наблюдатель не включаются при
            восстановлении. После него нужен новый захват входа.
          </p>
          <label>
            <input
              type="checkbox"
              checked={confirmed}
              disabled={busy}
              onChange={(e) => setConfirmed(e.target.checked)}
            />{" "}
            Заменить все текущие профили данными этой копии
          </label>
          <p>
            <button disabled={busy || !confirmed} onClick={restore}>
              Восстановить данные
            </button>
          </p>
        </div>
      )}
      {busy && <p role="status">Проверка и обработка резервной копии…</p>}
      {error && (
        <p className="banner error" role="alert">
          {error}
        </p>
      )}
      {message && (
        <p className="banner" role="status">
          {message}{" "}
          <button onClick={() => window.location.reload()}>
            Перезагрузить клиент
          </button>
        </p>
      )}
    </section>
  );
}
