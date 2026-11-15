import React,{useEffect,useMemo,useState} from 'react';
import {get} from './api';
import {allocateGoals} from './allocation.mjs';
import {progressionReference} from './progression.mjs';
const n=(v:number)=>v.toLocaleString('ru-RU');
export function Planner({profile,tick,data,catalog}:{profile:string;tick:number;data:any;catalog:any}) {
  const [goals,setGoals]=useState<any[]>([]),[draft,setDraft]=useState<any>(null),[error,setError]=useState(''),[reload,setReload]=useState(0),[busy,setBusy]=useState(false);
  useEffect(()=>{const c=new AbortController();get(`/api/v1/profiles/${profile}/goals`,c.signal).then(v=>setGoals(v.goals)).catch(e=>{if(!c.signal.aborted)setError(e.message);});return ()=>c.abort();},[profile,tick,reload]);
  const choices=useMemo(()=>['characters','weapons'].flatMap(collection=>(data.collections[collection]??[]).filter((r:any)=>!r.values.trial_avatar_info&&r.values.avatar_id!==10000134).map((r:any)=>({collection,...r}))),[data]);
  const name=(v:any)=>v.avatar_id?(catalog.avatarNames[v.avatar_id]??`Персонаж ${v.avatar_id}`):(catalog.names[v.item_id]??`Оружие ${v.item_id}`);
  const allocation=useMemo(()=>allocateGoals(goals,catalog,data),[goals,catalog,data]);
  const plans=allocation.plans;
  const item=(id:number)=>catalog.names[id]??`Предмет ${id}`;
  async function save(body:any) {
    setBusy(true);setError('');
    try {const r=await fetch(`/api/v1/profiles/${profile}/goals`,{method:'POST',headers:{'Content-Type':'application/json','X-Vision-Control':'1'},body:JSON.stringify(body)});const v=await r.json();if(!r.ok)throw Error(v.error?.message??'Не удалось сохранить цель.');setDraft(null);setReload(x=>x+1);}
    catch(e){setError(e instanceof Error?e.message:'Ошибка сохранения.');}finally{setBusy(false);}
  }
  function select(choice:any) {
    const existing=goals.find(g=>g.values.collection===choice.collection&&g.values.entity_key===choice.entity_key);
    if(existing){setDraft({...existing});return;}

    const v=choice.values;const ref=progressionReference(v,catalog);
    const current=Number(v.level??v.props?.['4001']?.ival??1),asc=Number(v.ascension??v.props?.['1002']?.ival??0);
    setDraft({values:{collection:choice.collection,entity_key:choice.entity_key,level:current,ascension:asc,skill_depot_id:v.avatar_id?Number(v.skill_depot_id):null,talents:Object.fromEntries((ref?.talents??[]).filter((t:any)=>v.skill_levels?.[t.skillId]!==undefined).map((t:any)=>[t.skillId,Number(v.skill_levels?.[t.skillId]??1)])),priority:goals.length},choice});
  }
  const draftRow=draft?.choice??choices.find(c=>c.collection===draft?.values.collection&&c.entity_key===draft?.values.entity_key);
  return <section>
    <h2>Цели прокачки</h2><p>Цели сохраняются для текущего профиля. Получение ресурсов и прокачка обновляют оставшуюся потребность автоматически.</p>
    {error&&<p role="alert">{error}</p>}
    <label>Добавить цель <select aria-label="Выбрать цель прокачки" value="" onChange={e=>{const c=choices.find(x=>`${x.collection}:${x.entity_key}`===e.target.value);if(c)select(c);}}><option value="">Персонаж или оружие…</option>{choices.map(c=><option key={`${c.collection}:${c.entity_key}`} value={`${c.collection}:${c.entity_key}`}>{name(c.values)} · {c.values.level??c.values.props?.['4001']?.ival??'?'} ур. · {c.entity_key.slice(-6)}</option>)}</select></label>
    {draft&&<form className="wish-pity-card" onSubmit={e=>{e.preventDefault();void save({goal_id:draft.goal_id,revision:draft.revision,values:draft.values});}}>
      <h3>{draftRow?name(draftRow.values):'Цель'}</h3>
      <label>Целевой уровень <input aria-label="Целевой уровень" type="number" min="1" max="90" value={draft.values.level} onChange={e=>setDraft({...draft,values:{...draft.values,level:Number(e.target.value)}})} /></label>
      <label>Возвышение <input aria-label="Целевое возвышение" type="number" min="0" max="6" value={draft.values.ascension} onChange={e=>setDraft({...draft,values:{...draft.values,ascension:Number(e.target.value)}})} /></label>
      <label>Приоритет (меньше — выше) <input aria-label="Приоритет цели" type="number" min="0" max="999" value={draft.values.priority} onChange={e=>setDraft({...draft,values:{...draft.values,priority:Number(e.target.value)}})} /></label>
      {Object.entries(draft.values.talents).map(([id,level])=><label key={id}>{catalog.skillNames[id]??`Талант ${id}`} · базовый уровень <input aria-label={`Целевой талант ${id}`} type="number" min="1" max="10" value={Number(level)} onChange={e=>setDraft({...draft,values:{...draft.values,talents:{...draft.values.talents,[id]:Number(e.target.value)}}})} /></label>)}
      {draft.values.skill_depot_id&&<small>Набор талантов: {draft.values.skill_depot_id}. Цель сохраняется для выбранной стихии.</small>}
      <p>Требования талантов могут повысить расчётную цель уровня и возвышения.</p>
      <button disabled={busy}>Сохранить цель</button> <button type="button" onClick={()=>setDraft(null)}>Отмена</button>
    </form>}
    {!goals.length&&<p>Пока нет целей. Выберите персонажа или конкретный экземпляр оружия.</p>}
    {goals.length>0&&<>
      <h3>Распределение ресурсов</h3>
      <p>Сначала ресурсы получают цели с меньшим числом приоритета. Выделение пересчитывается автоматически и не расходует предметы в игре.</p>
      {!catalog.crafting&&<p role="status">В выбранной версии справочника нет рецептов. Крафт не включён в расчёт.</p>}
      {allocation.partial&&<p role="status">Часть целей не удалось рассчитать. Для них материалы не выделены.</p>}
      <p>Книги и руда подбираются из оставшихся запасов. Если их недостаточно, недостающий опыт показан расчётным набором. Гарантированный крафт учитывает сырьё, мору и ранг приключений; случайные бонусы не включены.</p>
      <div className="table-wrap"><table><thead><tr><th>Ресурс</th><th>Нужно всего</th><th>Есть</th><th>Из запаса</th><th>Крафт</th><th>Не хватает</th></tr></thead><tbody>{allocation.materials.map(m=><tr key={m.id}><td>{item(m.id)}</td><td>{n(m.count)}</td><td>{m.owned===null?'неизвестно':n(m.owned)}</td><td>{m.allocated===null?'неизвестно':n(m.allocated)}</td><td>{n(m.crafted??0)}</td><td>{m.missing===null?'неизвестно':n(m.missing)}</td></tr>)}</tbody></table></div>
      {allocation.craftInputs.length>0&&<details className="craft-inputs"><summary>Сырьё выделено на крафт</summary><ul>{allocation.craftInputs.map(m=><li key={m.id}>{item(m.id)} × {n(m.count)} · в запасе {m.owned===null?'неизвестно':n(m.owned)}</li>)}</ul><p>Это сырьё уже исключено из запаса для остальных целей.</p></details>}
      <div className="wish-pity-grid planner-goals">{plans.map(({goal:g,plan:p,materials,mora,ready,crafts,craftMora},index)=><article className="wish-pity-card" key={g.goal_id}>
        <small>Очередь {index+1} · Приоритет {g.values.priority}</small><h3>{p.values?name(p.values):'Цель'}</h3><p>{p.complete?'Цель достигнута':p.status==='unknown'?p.reason:`Цель: ур. ${p.targetLevel}, возвышение ${p.targetAsc}`}</p>
        {p.status==='saved'&&<p>Расчёт по сохранённым данным.</p>}
        {p.requiredAdventureRank>0&&(p.currentRank===null||p.currentRank<p.requiredAdventureRank)&&<p>Для выполнения нужен ранг приключений {p.requiredAdventureRank}.</p>}
        {p.status!=='unknown'&&!p.complete&&<>
          <p className={`upgrade-status ${ready?'ready':'shortage'}`}>{ready?(crafts.length?'Можно выполнить после крафта':'Все материалы выделены'):materials.some(m=>m.missing===null)||mora!.missing===null?'Запасы не подтверждены':'Не все материалы выделены'}</p>
          <p>{p.experience>0&&<>Осталось опыта: {n(p.experience)}. </>}Мора: {n(mora!.count)}; выделено {mora!.allocated===null?'неизвестно':n(mora!.allocated)}; не хватает {mora!.missing===null?'неизвестно':n(mora!.missing)}.</p>
          <div className="table-wrap"><table><thead><tr><th>Ресурс</th><th>Нужно</th><th>Из запаса</th><th>Крафт</th><th>Не хватает</th></tr></thead><tbody>{materials.map(m=><tr key={m.id}><td>{item(m.id)}{m.uncertain&&' · запас опыта не подтверждён'}</td><td>{n(m.count)}</td><td>{m.allocated===null?'неизвестно':n(m.allocated)}</td><td>{n(m.crafted??0)}</td><td>{m.missing===null?'неизвестно':n(m.missing)}</td></tr>)}</tbody></table></div>
          {crafts.length>0&&<details className="craft-steps"><summary>Крафт: {n(craftMora)} моры</summary><ol>{crafts.map((step,i)=><li key={`${step.recipeId}:${i}`}>{item(step.inputId)} × {n(step.inputCount)} → {item(step.itemId)} × {n(step.count)}<br/><small>{n(step.mora)} моры</small></li>)}</ol><p>Стоимость крафта включена в общую мору цели. Предметы появятся в инвентаре после крафта в игре.</p></details>}
        </>}
        <button disabled={busy} onClick={()=>setDraft({...g})}>Изменить</button> <button disabled={busy} onClick={()=>void save({goal_id:g.goal_id,revision:g.revision,delete:true})}>Удалить</button>
      </article>)}</div>
    </>}
  </section>;
}
