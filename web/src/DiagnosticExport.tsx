import { tr, uiLocale } from "./i18n.mjs";
import { useState, useEffect, useRef } from "react";
const size = (value: any) => {
  const bytes = Number(value);
  const unit = bytes < 1024 ? "Б" : bytes < 1024 * 1024 ? "КБ" : "МБ";
  const amount =
    unit === "Б" ? bytes : unit === "КБ" ? bytes / 1024 : bytes / 1024 / 1024;
  return `${amount.toLocaleString(uiLocale, { maximumFractionDigits: 1 })} ${tr(unit)}`;
};
export function DiagnosticExport({ capture }: { capture: string }) {
  const [packets, setPackets] = useState(false),
    [description, setDescription] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [ready, setReady] = useState<any>(null),
    [started, setStarted] = useState(false);
  const generation = useRef(0);
  useEffect(() => {
    generation.current++;
    setReady(null);
    setError("");
    setStarted(false);
    setBusy(false);
  }, [capture]);
  useEffect(
    () => () => {
      generation.current++;
    },
    [],
  );
  async function prepare() {
    if (!capture || busy) return;
    const version = ++generation.current;
    setBusy(true);
    setError("");
    setReady(null);
    setStarted(false);
    try {
      const response = await fetch("/api/v1/diagnostics/export", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-Vision-Control": "1",
        },
        body: JSON.stringify({
          capture_id: capture,
          include_packets: packets,
          description,
        }),
      });
      const result = await response.json();
      if (!response.ok)
        throw new Error(result.error?.message ?? tr("Не удалось создать архив"));
      if (version === generation.current) setReady(result);
    } catch (e) {
      if (version === generation.current)
        setError(
          tr("Не удалось подготовить архив. {0}", e instanceof Error ? e.message : ""),
        );
    } finally {
      if (version === generation.current) setBusy(false);
    }
  }
  return (
    <section className="diagnostic-export" aria-label={tr("Экспорт диагностики")}>
      <h2>{tr("Экспорт для баг-репорта")}</h2>
      <p>{tr("Архив выбранного захвата: журналы приложения и захвата, сохранённые снимки, события из БД и сведения о версии справочника. База данных целиком и файлы с ключами не включаются.")}</p>
      {!capture && (
        <p className="footnote">{tr("Сначала выберите захват в списке выше.")}</p>
      )}
      <label className="diagnostic-description">{tr("Описание проблемы")}<textarea
          aria-label={tr("Описание проблемы")}
          maxLength={2000}
          rows={3}
          value={description}
          disabled={busy}
          onChange={(e) => {
            setDescription(e.target.value);
            setReady(null);
          }}
          placeholder={tr("Что произошло, что ожидалось и примерное время события")}
        />
      </label>
      <label className="diagnostic-packets">
        <input
          type="checkbox"
          checked={packets}
          disabled={busy}
          onChange={(e) => {
            setPackets(e.target.checked);
            setReady(null);
          }}
        />{tr("Включить сохранённые raw-пакеты")}</label>
      <p className="footnote">{tr("Архив содержит UID и игровые данные. Raw-пакеты не обезличиваются и могут содержать сетевые и сессионные данные. Перед отправкой архива другим людям проверьте его содержимое. Ничего не отправляется автоматически.")}</p>
      <p className="footnote">{tr("Захват можно не останавливать. В архив попадут только уже записанные данные; вытесненные сегменты восстановить нельзя. Для raw-пакетов без начала записи повторное декодирование может быть недоступно.")}</p>
      <button disabled={!capture || busy} onClick={() => void prepare()}>
        {busy ? tr("Готовим архив…") : tr("Подготовить диагностический ZIP")}
      </button>
      {error && <p role="alert">{error}</p>}
      {started && (
        <p role="status">{tr("Загрузка началась. Для повторного скачивания подготовьте архив заново.")}</p>
      )}
      {ready && (
        <div className="diagnostic-ready" role="status">
          <strong>{tr("Архив готов · ")}{size(ready.bytes)}</strong>
          <p>{tr("Файлов: ")}{ready.manifest.files.length}{tr(". Raw-пакеты:")}{" "}
            {ready.manifest.raw.included
              ? tr("{0} сегм.", ready.manifest.raw.segments)
              : tr("не включены")}
            .
          </p>
          {ready.manifest.warnings.length > 0 && (
            <p>{tr("В составе есть ограничения или пропуски — подробности ниже и в manifest.json.")}</p>
          )}
          <a
            href={ready.download_url}
            download={ready.filename}
            onClick={() => {
              setStarted(true);
              setTimeout(() => setReady(null), 0);
            }}
          >{tr("Скачать ZIP")}</a>
          <small>{tr("Ссылка действует 15 минут и доступна для одного скачивания.")}</small>
          <details>
            <summary>{tr("Состав и ограничения архива")}</summary>
            <ul>
              {ready.manifest.files.map((f: any) => (
                <li key={f.path}>
                  {f.path} · {size(f.bytes)}
                </li>
              ))}
            </ul>
            {ready.manifest.warnings.map((w: string) => (
              <p key={w}>{w}</p>
            ))}
          </details>
        </div>
      )}
    </section>
  );
}
