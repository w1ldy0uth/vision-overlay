import React, {useMemo} from 'react';
import {analyzeWishes} from './wish-pity.mjs';
import reference from './data/wish-banners.json';

export function WishPity({data,catalog}:{data:any;catalog:any}) {
  const meta=data.last_import?.metadata??{};
  const stats=useMemo(()=>analyzeWishes(data.records,{catalog,periods:reference.rows,coverage:meta,zone:meta.region_time_zone}),[data,catalog]);
  return <section className="wish-pity">
    <h3>Pity и гарантии · 5★</h3>
    <p>События персонажей I и II имеют общий счётчик. {data.live_enabled?"Новые молитвы учитываются из захвата.":"Live-добавление выключено до подтверждения синхронизации."}</p>
    <div className="wish-pity-grid">{stats.map(s=><article className="wish-pity-card" key={s.id}>
      <h4>{s.name}</h4><small>Сохранено: {s.total} · Получено 5★: {s.five}</small>
      {s.cap===null?<p>Для баннера новичка pity и событийный гарант не рассчитываются.</p>:<>
        <p><strong>{s.total?`${s.exact?'':'≥ '}${s.pity} / ${s.cap}`:'Нет данных'}</strong><br/><small>Молитв после последнего 5★</small></p>
        {s.remaining!==null?<p>До гарантированного 5★: не более <b>{s.remaining}</b> молитв.</p>:s.total>0&&<p>Точный остаток неизвестен: нет подтверждённого непрерывного участка от последнего 5★.</p>}
        {s.invalid&&<p role="alert">Счётчик превышает допустимый предел. Нужна проверка истории.</p>}
        {s.guarantee!=='not_applicable'&&<p><b>{s.guarantee==='guaranteed'?'Следующий 5★ — гарант':s.guarantee==='not_guaranteed'?'Следующий 5★ — 50/50':'Гарантия неизвестна'}</b></p>}
        {s.last&&<small>Последний 5★: {s.last.name} · {s.last.time}</small>}
      </>}
    </article>)}</div>
    <details><summary>Правила и точность расчёта</summary>
      <p>У событий персонажей — предел 90 молитв до 5★, у оружия — 80, у стандартного баннера и «Молитвы хроник» — 90. После любого 5★ счётчик своей группы сбрасывается. «≥» означает только подтверждённое число сохранённых молитв; неизвестная предыстория не принимается за ноль.</p>
      <p>Отсутствие событийного гаранта не означает точную вероятность 50% следующего результата: механика «Пленительный блеск» учитывается отдельно. Её скрытое состояние и вероятность по этой истории не вычисляются. Soft pity также не моделируется.</p>
      <p>Состав баннера определяется по дате и часовому поясу API. Без однозначного сопоставления состояние гарантии остаётся неизвестным. Курс оружия и хроник нельзя восстановить по одному списку наград; их очки судьбы не переносятся между периодами.</p>
      <p><a href="https://support.hoyoverse.com/hc/en-us/articles/50333940684953-How-does-the-Wish-guarantee-system-work" target="_blank" rel="noreferrer">Правила HoYoverse</a> · <a href="https://genshin.hoyoverse.com/en/news/detail/122695" target="_blank" rel="noreferrer">Молитва хроник</a> · <a href={`https://github.com/MadeBaruna/paimon-moe/blob/${reference.revision}/src/data/banners.js`} target="_blank" rel="noreferrer">Данные составов баннеров</a></p>
      <small>Справочник составов: {reference.revision.slice(0,12)}. При неизвестном новом периоде pity продолжает считаться, событийная гарантия не угадывается.</small>
    </details>
  </section>;
}
