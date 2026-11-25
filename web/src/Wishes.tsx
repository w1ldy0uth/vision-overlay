import { gameText, tr, uiLocale } from "./i18n.mjs";
import {wishRecords} from "./wish-live.mjs";
import { WishPity } from "./WishPity";
import React, { useEffect, useMemo, useState } from "react";
import { get } from "./api";

const banners: Record<string,string> = {"100":tr("Новичка"),"200":tr("Стандартная"),"301":tr("Событие персонажа I"),"400":tr("Событие персонажа II"),"302":tr("Событие оружия"),"500":tr("Хроник")};
export function Wishes({profile,tick,catalog}:{profile:string;tick:number;catalog:any}) {
  const [data,setData] = useState<any>(null), [error,setError] = useState(""), [message,setMessage] = useState("");
  const [busy,setBusy] = useState(false), [reload,setReload] = useState(0);
  const [search,setSearch] = useState(""), [banner,setBanner] = useState("all"), [rarity,setRarity] = useState("all"), [limit,setLimit] = useState(100);
  useEffect(()=>{
    const c = new AbortController();
    get(`/api/v1/profiles/${profile}/wishes`,c.signal).then(v=>{setData(v);setError("");}).catch(e=>{if(!c.signal.aborted)setError(e.message);});
    return ()=>c.abort();
  },[profile,tick,reload]);
  const combined=useMemo(()=>data?wishRecords(data,catalog).map((row:any)=>({...row,name:gameText(row.name)})):[],[data,catalog]);
  const rows = useMemo(()=> combined.filter((r:any)=>
    (banner==="all" || (banner==="characters" ? ["301","400"].includes(r.gacha_type) : r.gacha_type===banner)) &&
    (rarity==="all" || r.rank_type===rarity) &&
    `${r.name} ${r.id} ${r.item_id??""}`.toLowerCase().includes(search.trim().toLowerCase())),[combined,banner,rarity,search]);
  async function upload(file?: File) {
    if(!file)return;
    setBusy(true);setError("");setMessage("");
    try {
      if(file.size>32*1024*1024)throw new Error(tr("Архив превышает 32 МБ."));
      const body = JSON.parse(await file.text());
      const r = await fetch(`/api/v1/profiles/${profile}/wishes`,{method:"POST",headers:{"Content-Type":"application/json","X-Vision-Control":"1"},body:JSON.stringify(body)});
      const result = await r.json();
      if(!r.ok)throw new Error(result.error?.message??tr("Не удалось импортировать историю."));
      setMessage(tr("Получено записей: {0}. Добавлено новых: {1}.", result.received, result.added));
      setReload(n=>n+1);
    } catch(e) {setError(e instanceof Error?e.message:tr("Не удалось прочитать архив."));}
    finally {setBusy(false);}
  }
  async function sync() {
    setError("");
    try {
      const r=await fetch(`/api/v1/profiles/${profile}/wishes/sync`,{method:"POST",headers:{"X-Vision-Control":"1"}});
      const result=await r.json();if(!r.ok)throw new Error(result.error?.message??tr("Не удалось запустить синхронизацию."));
      setReload(n=>n+1);
    }catch(e){setError(e instanceof Error?e.message:tr("Ошибка синхронизации."));}
  }
  const syncing=["syncing","requested"].includes(data?.status);
  const syncLabels:Record<string,string>={syncing:tr("Синхронизируем историю…"),requested:tr("Синхронизация запланирована…"),synced:tr("История обновлена"),needs_access:tr("Нужно открыть историю в игре"),failed:tr("Не удалось обновить историю"),cancelled:tr("Синхронизация прервана при смене сессии"),needs_sync:tr("История может быть неполной"),not_imported:tr("История ещё не импортирована")};
  return <section className="panel wishes-panel">
    <h2>{tr("История молитв")}</h2>
    <p>{tr("Импортируйте JSON-архив, сохранённый утилитой истории Vision Overlay. Записи сохраняются для выбранного UID и сервера; повторный импорт добавляет только новые молитвы.")}</p>
    <label>{tr("Импорт архива ")}<input aria-label={tr("Импорт истории молитв")} type="file" accept=".json,application/json" disabled={busy} onChange={e=>{void upload(e.target.files?.[0]);e.target.value="";}} /></label>
    {busy && <p role="status">{tr("Импортируем историю…")}</p>}
    {message && <p role="status">{message}</p>}
    {error && <p role="alert">{error}</p>}
    {data && <>
      <aside className="notice" role="status"><strong>{syncLabels[data.status]??tr("История может быть неполной")}</strong>
        <p>{data.status==="synced" ? (data.live_enabled?tr("Новые молитвы добавляются из захвата автоматически."):tr("Доступная история получена. Для live-обновлений нужен активный захват с поддержкой молитв.")) : syncing ? tr("Загружаем доступные страницы для текущего UID и сервера. Сохранённая история остаётся доступной.") : data.status==="failed" ? tr("Не удалось завершить загрузку истории. Проверьте доступность истории в игре и настройки VPN, затем повторите синхронизацию. Сохранённая история доступна, live-добавление выключено.") : data.status==="needs_access" ? tr("Откройте «Молитва → История» в игре, дождитесь загрузки списка и нажмите «Синхронизировать». Сохранённая история может не содержать последующие молитвы; live-добавление выключено.") : tr("После последней выгрузки могли быть совершены новые молитвы. Сохранённые записи доступны, live-добавление выключено. При входе в игру приложение автоматически пробует обновить историю; при ошибке можно повторить запрос.")}</p>
        {data.last_import && <small>{tr("Выгрузка: ")}{new Date(Number(data.last_import.fetched_ms)).toLocaleString(uiLocale)}{tr(" · Импорт: ")}{new Date(Number(data.last_import.imported_ms)).toLocaleString(uiLocale)}</small>}
        <p><button disabled={syncing||busy} onClick={()=>void sync()}>{syncing?tr("Синхронизация…"):tr("Синхронизировать")}</button></p>
      </aside>
      <WishPity data={{...data,records:combined}} catalog={catalog} />
      <div className="toolbar">
        <input aria-label={tr("Поиск молитв")} placeholder={tr("Название или ID…")} value={search} onChange={e=>{setSearch(e.target.value);setLimit(100);}} />
        <select aria-label={tr("Баннер молитв")} value={banner} onChange={e=>{setBanner(e.target.value);setLimit(100);}}><option value="all">{tr("Все баннеры")}</option><option value="characters">{tr("События персонажей I + II")}</option>{Object.entries(banners).map(([id,label])=><option key={id} value={id}>{label}</option>)}</select>
        <select aria-label={tr("Редкость молитв")} value={rarity} onChange={e=>{setRarity(e.target.value);setLimit(100);}}><option value="all">{tr("Любая редкость")}</option>{["5","4","3"].map(n=><option key={n} value={n}>{n}★</option>)}</select>
      </div>
      <p>{tr("Всего: ")}{combined.length}{tr(". Найдено: ")}{rows.length}{tr(". Время API — по игровому серверу, время захвата — местное.")}</p>
      <div className="table-wrap"><table><thead><tr><th>{tr("Время")}</th><th>{tr("Результат")}</th><th>{tr("Редкость")}</th><th>{tr("Баннер")}</th><th>{tr("Источник")}</th><th>ID</th></tr></thead><tbody>{rows.slice(0,limit).map((r:any)=><tr key={r.id}><td>{r.display_time??r.time}</td><td>{r.name}</td><td>{r.rank_type?`${r.rank_type}★`:"—"}</td><td>{banners[r.gacha_type]}</td><td>{r.live?tr("Захват · ожидает API"):tr("История API")}</td><td>{r.live?"—":r.id}</td></tr>)}</tbody></table></div>
      {!rows.length && <p>{tr("Нет молитв по выбранным условиям.")}</p>}
      {rows.length>limit && <button onClick={()=>setLimit(n=>n+100)}>{tr("Показать ещё")}</button>}
    </>}
  </section>;
}
