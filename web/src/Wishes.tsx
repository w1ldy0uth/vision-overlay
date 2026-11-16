import {wishRecords} from "./wish-live.mjs";
import { WishPity } from "./WishPity";
import React, { useEffect, useMemo, useState } from "react";
import { get } from "./api";

const banners: Record<string,string> = {"100":"Новичка","200":"Стандартная","301":"Событие персонажа I","400":"Событие персонажа II","302":"Событие оружия","500":"Хроник"};
export function Wishes({profile,tick,catalog}:{profile:string;tick:number;catalog:any}) {
  const [data,setData] = useState<any>(null), [error,setError] = useState(""), [message,setMessage] = useState("");
  const [busy,setBusy] = useState(false), [reload,setReload] = useState(0);
  const [search,setSearch] = useState(""), [banner,setBanner] = useState("all"), [rarity,setRarity] = useState("all"), [limit,setLimit] = useState(100);
  useEffect(()=>{
    const c = new AbortController();
    get(`/api/v1/profiles/${profile}/wishes`,c.signal).then(v=>{setData(v);setError("");}).catch(e=>{if(!c.signal.aborted)setError(e.message);});
    return ()=>c.abort();
  },[profile,tick,reload]);
  const combined=useMemo(()=>data?wishRecords(data,catalog):[],[data,catalog]);
  const rows = useMemo(()=> combined.filter((r:any)=>
    (banner==="all" || (banner==="characters" ? ["301","400"].includes(r.gacha_type) : r.gacha_type===banner)) &&
    (rarity==="all" || r.rank_type===rarity) &&
    `${r.name} ${r.id} ${r.item_id??""}`.toLowerCase().includes(search.trim().toLowerCase())),[combined,banner,rarity,search]);
  async function upload(file?: File) {
    if(!file)return;
    setBusy(true);setError("");setMessage("");
    try {
      if(file.size>32*1024*1024)throw new Error("Архив превышает 32 МБ.");
      const body = JSON.parse(await file.text());
      const r = await fetch(`/api/v1/profiles/${profile}/wishes`,{method:"POST",headers:{"Content-Type":"application/json","X-Vision-Control":"1"},body:JSON.stringify(body)});
      const result = await r.json();
      if(!r.ok)throw new Error(result.error?.message??"Не удалось импортировать историю.");
      setMessage(`Получено записей: ${result.received}. Добавлено новых: ${result.added}.`);
      setReload(n=>n+1);
    } catch(e) {setError(e instanceof Error?e.message:"Не удалось прочитать архив.");}
    finally {setBusy(false);}
  }
  async function sync() {
    setError("");
    try {
      const r=await fetch(`/api/v1/profiles/${profile}/wishes/sync`,{method:"POST",headers:{"X-Vision-Control":"1"}});
      const result=await r.json();if(!r.ok)throw new Error(result.error?.message??"Не удалось запустить синхронизацию.");
      setReload(n=>n+1);
    }catch(e){setError(e instanceof Error?e.message:"Ошибка синхронизации.");}
  }
  const syncing=["syncing","requested"].includes(data?.status);
  const syncLabels:Record<string,string>={syncing:"Синхронизируем историю…",requested:"Синхронизация запланирована…",synced:"История обновлена",needs_access:"Нужно открыть историю в игре",failed:"Не удалось обновить историю",cancelled:"Синхронизация прервана при смене сессии",needs_sync:"История может быть неполной",not_imported:"История ещё не импортирована"};
  return <section className="panel wishes-panel">
    <h2>История молитв</h2>
    <p>Импортируйте JSON-архив, сохранённый утилитой истории Vision Overlay. Записи сохраняются для выбранного UID и сервера; повторный импорт добавляет только новые молитвы.</p>
    <label>Импорт архива <input aria-label="Импорт истории молитв" type="file" accept=".json,application/json" disabled={busy} onChange={e=>{void upload(e.target.files?.[0]);e.target.value="";}} /></label>
    {busy && <p role="status">Импортируем историю…</p>}
    {message && <p role="status">{message}</p>}
    {error && <p role="alert">{error}</p>}
    {data && <>
      <aside className="notice" role="status"><strong>{syncLabels[data.status]??"История может быть неполной"}</strong>
        <p>{data.status==="synced" ? (data.live_enabled?"Новые молитвы добавляются из захвата автоматически.":"Доступная история получена. Для live-обновлений нужен активный захват с поддержкой молитв.") : syncing ? "Загружаем доступные страницы для текущего UID и сервера. Сохранённая история остаётся доступной." : data.status==="failed" ? "Не удалось завершить загрузку истории. Проверьте доступность истории в игре и настройки VPN, затем повторите синхронизацию. Сохранённая история доступна, live-добавление выключено." : data.status==="needs_access" ? "Откройте «Молитва → История» в игре, дождитесь загрузки списка и нажмите «Синхронизировать». Сохранённая история может не содержать последующие молитвы; live-добавление выключено." : "После последней выгрузки могли быть совершены новые молитвы. Сохранённые записи доступны, live-добавление выключено. При входе в игру приложение автоматически пробует обновить историю; при ошибке можно повторить запрос."}</p>
        {data.last_import && <small>Выгрузка: {new Date(Number(data.last_import.fetched_ms)).toLocaleString("ru-RU")} · Импорт: {new Date(Number(data.last_import.imported_ms)).toLocaleString("ru-RU")}</small>}
        <p><button disabled={syncing||busy} onClick={()=>void sync()}>{syncing?"Синхронизация…":"Синхронизировать"}</button></p>
      </aside>
      <WishPity data={{...data,records:combined}} catalog={catalog} />
      <div className="toolbar">
        <input aria-label="Поиск молитв" placeholder="Название или ID…" value={search} onChange={e=>{setSearch(e.target.value);setLimit(100);}} />
        <select aria-label="Баннер молитв" value={banner} onChange={e=>{setBanner(e.target.value);setLimit(100);}}><option value="all">Все баннеры</option><option value="characters">События персонажей I + II</option>{Object.entries(banners).map(([id,label])=><option key={id} value={id}>{label}</option>)}</select>
        <select aria-label="Редкость молитв" value={rarity} onChange={e=>{setRarity(e.target.value);setLimit(100);}}><option value="all">Любая редкость</option>{["5","4","3"].map(n=><option key={n} value={n}>{n}★</option>)}</select>
      </div>
      <p>Всего: {combined.length}. Найдено: {rows.length}. Время API — по игровому серверу, время захвата — местное.</p>
      <div className="table-wrap"><table><thead><tr><th>Время</th><th>Результат</th><th>Редкость</th><th>Баннер</th><th>Источник</th><th>ID</th></tr></thead><tbody>{rows.slice(0,limit).map((r:any)=><tr key={r.id}><td>{r.display_time??r.time}</td><td>{r.name}</td><td>{r.rank_type?`${r.rank_type}★`:"—"}</td><td>{banners[r.gacha_type]}</td><td>{r.live?"Захват · ожидает API":"История API"}</td><td>{r.live?"—":r.id}</td></tr>)}</tbody></table></div>
      {!rows.length && <p>Нет молитв по выбранным условиям.</p>}
      {rows.length>limit && <button onClick={()=>setLimit(n=>n+100)}>Показать ещё</button>}
    </>}
  </section>;
}
