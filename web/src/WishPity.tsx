import { gameText, tr } from "./i18n.mjs";
import React, {useMemo} from 'react';
import {analyzeWishes} from './wish-pity.mjs';
import reference from './data/wish-banners.json';

export function WishPity({data,catalog}:{data:any;catalog:any}) {
  const meta=data.last_import?.metadata??{};
  const stats=useMemo(()=>analyzeWishes(data.records,{catalog:catalog.originalWishNames??catalog,periods:reference.rows,coverage:meta,zone:meta.region_time_zone}),[data,catalog]);
  return <section className="wish-pity">
    <h3>{tr("Pity и гарантии · 5★")}</h3>
    <p>{tr("События персонажей I и II имеют общий счётчик. ")}{data.live_enabled?tr("Новые молитвы учитываются из захвата."):tr("Live-добавление выключено до подтверждения синхронизации.")}</p>
    <div className="wish-pity-grid">{stats.map(s=><article className="wish-pity-card" key={s.id}>
      <h4>{s.name}</h4><small>{tr("Сохранено: ")}{s.total}{tr(" · Получено 5★: ")}{s.five}</small>
      {s.cap===null?<p>{tr("Для баннера новичка pity и событийный гарант не рассчитываются.")}</p>:<>
        <p><strong>{s.total?`${s.exact?'':'≥ '}${s.pity} / ${s.cap}`:tr("Нет данных")}</strong><br/><small>{tr("Молитв после последнего 5★")}</small></p>
        {s.remaining!==null?<p>{tr("До гарантированного 5★: не более ")}<b>{s.remaining}</b>{tr(" молитв.")}</p>:s.total>0&&<p>{tr("Точный остаток неизвестен: нет подтверждённого непрерывного участка от последнего 5★.")}</p>}
        {s.invalid&&<p role="alert">{tr("Счётчик превышает допустимый предел. Нужна проверка истории.")}</p>}
        {s.guarantee!=='not_applicable'&&<p><b>{s.guarantee==='guaranteed'?tr("Следующий 5★ — гарант"):s.guarantee==='not_guaranteed'?tr("Следующий 5★ — 50/50"):tr("Гарантия неизвестна")}</b></p>}
        {s.last&&<small>{tr("Последний 5★: ")}{gameText(s.last.name)} · {s.last.time}</small>}
      </>}
    </article>)}</div>
    <details><summary>{tr("Правила и точность расчёта")}</summary>
      <p>{tr("У событий персонажей — предел 90 молитв до 5★, у оружия — 80, у стандартного баннера и «Молитвы хроник» — 90. После любого 5★ счётчик своей группы сбрасывается. «≥» означает только подтверждённое число сохранённых молитв; неизвестная предыстория не принимается за ноль.")}</p>
      <p>{tr("Отсутствие событийного гаранта не означает точную вероятность 50% следующего результата: механика «Пленительный блеск» учитывается отдельно. Её скрытое состояние и вероятность по этой истории не вычисляются. Soft pity также не моделируется.")}</p>
      <p>{tr("Состав баннера определяется по дате и часовому поясу API. Без однозначного сопоставления состояние гарантии остаётся неизвестным. Курс оружия и хроник нельзя восстановить по одному списку наград; их очки судьбы не переносятся между периодами.")}</p>
      <p><a href="https://support.hoyoverse.com/hc/en-us/articles/50333940684953-How-does-the-Wish-guarantee-system-work" target="_blank" rel="noreferrer">{tr("Правила HoYoverse")}</a> · <a href="https://genshin.hoyoverse.com/en/news/detail/122695" target="_blank" rel="noreferrer">{tr("Молитва хроник")}</a> · <a href={`https://github.com/MadeBaruna/paimon-moe/blob/${reference.revision}/src/data/banners.js`} target="_blank" rel="noreferrer">{tr("Данные составов баннеров")}</a></p>
      <small>{tr("Справочник составов: ")}{reference.revision.slice(0,12)}{tr(". При неизвестном новом периоде pity продолжает считаться, событийная гарантия не угадывается.")}</small>
    </details>
  </section>;
}
