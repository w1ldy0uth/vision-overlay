import React,{useEffect,useMemo,useState} from 'react';
import {get} from './api';
import { LevelTarget } from './LevelTarget';
import { equippedWeapon, maximumWeaponLevel } from './goal-targets.mjs';
import { GoalPicker, type GoalChoice } from './GoalPicker';
import {groupPlannerResources} from './planner-resources.mjs';
import {allocateGoals} from './allocation.mjs';
import {progressionReference} from './progression.mjs';
const n=(v:number)=>v.toLocaleString('ru-RU');
export function Planner({profile,tick,data,catalog,requestedGoal,onRequestHandled}:{profile:string;tick:number;data:any;catalog:any;requestedGoal?:GoalChoice;onRequestHandled:()=>void}) {
  const [goals,setGoals]=useState<any[]>([]),[draft,setDraft]=useState<any>(null),[error,setError]=useState(''),[reload,setReload]=useState(0),[busy,setBusy]=useState(false);
  const [pickerOpen,setPickerOpen]=useState(false),[hideCollected,setHideCollected]=useState(false),[loaded,setLoaded]=useState(false);
  const [focusedGoal,setFocusedGoal]=useState<string|null>(null);
  const goalCards=React.useRef(new Map<string,HTMLElement>());
  const draftForm=React.useRef<HTMLFormElement>(null);
  const matches=(goal:any, choice:GoalChoice)=>goal.values.collection===choice.collection&&goal.values.entity_key===choice.entity_key;
  const visibleMaterial=(m:{missing:number|null;crafted?:number})=>!hideCollected||m.missing!==0||Boolean(m.crafted);
  useEffect(()=>{const c=new AbortController();get(`/api/v1/profiles/${profile}/goals`,c.signal).then(v=>{setGoals(v.goals);setLoaded(true);}).catch(e=>{if(!c.signal.aborted)setError(e.message);});return ()=>c.abort();},[profile,tick,reload]);
  const choices=useMemo<GoalChoice[]>(()=>(['characters','weapons'] as const).flatMap(collection=>(data.collections[collection]??[]).filter((r:any)=>!r.values.trial_avatar_info&&r.values.avatar_id!==10000134).map((r:any)=>({collection,...r}))),[data]);
  const name=(v:any)=>v.avatar_id?(catalog.avatarNames[v.avatar_id]??`Персонаж ${v.avatar_id}`):(catalog.names[v.item_id]??`Оружие ${v.item_id}`);
  const allocation=useMemo(()=>allocateGoals(goals,catalog,data),[goals,catalog,data]);
  const plans=allocation.plans;
  const item=(id:number)=>catalog.names[id]??`Предмет ${id}`;
  async function save(body:any) {
    setBusy(true);setError('');
    try {const r=await fetch(`/api/v1/profiles/${profile}/goals`,{method:'POST',headers:{'Content-Type':'application/json','X-Vision-Control':'1'},body:JSON.stringify(body)});const v=await r.json();if(!r.ok)throw Error(v.error?.message??'Не удалось сохранить цель.');setDraft(null);setLoaded(false);setReload(x=>x+1);}
    catch(e){setError(e instanceof Error?e.message:'Ошибка сохранения.');}finally{setBusy(false);}
  }
  function select(choice:GoalChoice) {
    const existing=goals.find(g=>matches(g,choice));
    if(existing){setDraft({...existing});return;}

    const v=choice.values;const ref=progressionReference(v,catalog);
    const current=Number(v.level??v.props?.['4001']?.ival??1),asc=Number(v.ascension??v.props?.['1002']?.ival??0);
    setDraft({values:{collection:choice.collection,entity_key:choice.entity_key,level:current,ascension:asc,skill_depot_id:v.avatar_id?Number(v.skill_depot_id):null,talents:Object.fromEntries((ref?.talents??[]).filter((t:any)=>v.skill_levels?.[t.skillId]!==undefined).map((t:any)=>[t.skillId,Number(v.skill_levels?.[t.skillId]??1)])),priority:goals.length},choice});
  }
  useEffect(()=>{
    if(!loaded||!requestedGoal)return;
    const choice=choices.find(row=>row.collection===requestedGoal.collection&&row.entity_key===requestedGoal.entity_key);
    const existing=goals.find(goal=>matches(goal,requestedGoal));
    const linked=requestedGoal.collection==='weapons' && plans.find(({linkedFrom})=>
      linkedFrom&&equippedWeapon(data,linkedFrom.values.entity_key)?.entity_key===requestedGoal.entity_key);
    if(existing||linked){
      setDraft(null);
      setFocusedGoal((existing??(linked&&linked.goal)).goal_id);
    } else if(choice)select(choice);
    else setError('Цель больше не доступна в текущем профиле.');
    onRequestHandled();
  },[loaded,requestedGoal,choices,onRequestHandled]);
  useEffect(()=>{
    if(!focusedGoal)return;
    const card=goalCards.current.get(focusedGoal);
    card?.focus({preventScroll:true});
    card?.scrollIntoView({block:'start'});
    setFocusedGoal(null);
  },[focusedGoal]);
  const draftKey=draft?.values.entity_key;
  useEffect(()=>{if(draftKey)draftForm.current?.querySelector('summary')?.focus();},[draftKey]);
  const draftRow=draft?.choice??choices.find(c=>c.collection===draft?.values.collection&&c.entity_key===draft?.values.entity_key);
  const characterDraft=draft?.values.collection==='characters';
  const weapon=characterDraft?equippedWeapon(data,draft.values.entity_key):undefined;
  const change=(patch:Record<string,unknown>)=>setDraft((d:any)=>({...d,values:{...d.values,...patch}}));
  function toggleWeapon(enabled:boolean) {
    if(enabled) {
      change({equipped_weapon:{level:Number(weapon?.values.level??1),ascension:Number(weapon?.values.ascension??0)}});
    } else {
      const values={...draft.values};
      delete values.equipped_weapon;
      setDraft({...draft,values});
    }
  }
  return <section>
    {error&&<p role="alert">{error}</p>}
    <button className="planner-picker-button" type="button" disabled={!loaded||busy} onClick={()=>setPickerOpen(true)}>{draft?"Изменить цель":"Добавить цель"}</button>
    {pickerOpen&&<GoalPicker choices={choices.filter(choice=>!goals.some(goal=>matches(goal,choice)))}
      catalog={catalog} characters={data.collections.characters??[]} name={name}
      onClose={()=>setPickerOpen(false)} onSelect={choice=>{setPickerOpen(false);select(choice);}} />}
    {draft&&<form ref={draftForm} className="wish-pity-card" onSubmit={e=>{e.preventDefault();void save({goal_id:draft.goal_id,revision:draft.revision,values:draft.values});}}>
      <h3>Цели прокачки</h3>
      {draftRow&&<h4>{name(draftRow.values)}</h4>}
      <div className="goal-parameters">
        {characterDraft&&<section>
          <h4>Возвышение персонажа</h4>
          <LevelTarget value={draft.values} onChange={change} />
        </section>}
        {characterDraft&&Object.keys(draft.values.talents).length>0&&<section>
          <h4>Таланты</h4>
          {Object.entries(draft.values.talents).map(([id,level])=><label className="goal-talent" key={id}>
            {catalog.skillNames[id]??`Талант ${id}`}
            <input aria-label={`Целевой базовый уровень: ${catalog.skillNames[id]??id}`} type="number" min="1" max="10"
              value={Number(level)} onChange={e=>change({talents:{...draft.values.talents,[id]:e.target.valueAsNumber}})} />
          </label>)}
          <small>Требования талантов могут повысить расчётный уровень и возвышение.</small>
        </section>}
        <section>
          <h4>Оружие</h4>
          {characterDraft&&<label className="planner-hide-collected">
            <input type="checkbox" checked={!!draft.values.equipped_weapon} onChange={e=>toggleWeapon(e.target.checked)} /> Экипированное оружие
          </label>}
          {characterDraft&&draft.values.equipped_weapon&&<>
            <small>{weapon?name(weapon.values):'Экипированное оружие ещё не определено.'}</small>
            <LevelTarget value={draft.values.equipped_weapon} onChange={target=>change({equipped_weapon:target})} />
          </>}
          {!characterDraft&&<LevelTarget value={draft.values} max={maximumWeaponLevel(draftRow,catalog)} onChange={change} />}
        </section>
        <label>Приоритет (меньше — выше) <input aria-label="Приоритет цели" type="number" min="0" max="999" value={draft.values.priority} onChange={e=>change({priority:Number(e.target.value)})} /></label>
      </div>

      <button disabled={busy}>Сохранить цель</button> <button type="button" onClick={()=>setDraft(null)}>Отмена</button>
    </form>}
    {!goals.length&&<p>Пока нет целей. Выберите персонажа или конкретный экземпляр оружия.</p>}
    {goals.length>0&&<>
      <h3>Распределение ресурсов</h3>
      <label className="planner-hide-collected"><input type="checkbox" checked={hideCollected} onChange={e=>setHideCollected(e.target.checked)} /> Скрыть собранное</label>
      <p>Сначала ресурсы получают цели с меньшим числом приоритета. Выделение пересчитывается автоматически и не расходует предметы в игре.</p>
      {!catalog.crafting&&<p role="status">В выбранной версии справочника нет рецептов. Крафт не включён в расчёт.</p>}
      {allocation.partial&&<p role="status">Часть целей не удалось рассчитать. Для них материалы не выделены.</p>}
      <p>Книги и руда подбираются из оставшихся запасов. Если их недостаточно, недостающий опыт показан расчётным набором. Гарантированный крафт учитывает сырьё, мору и ранг приключений; случайные бонусы не включены.</p>
      {groupPlannerResources(allocation.materials.filter(visibleMaterial),catalog).map(group=><details className="planner-resource-group" key={group.key} open>
        <summary>{group.title}</summary>
      <div className="table-wrap"><table><thead><tr><th>Ресурс</th><th>Нужно всего</th><th>Есть</th><th>Из запаса</th><th>Крафт</th><th>Не хватает</th></tr></thead><tbody>{group.rows.map(m=><tr key={m.id}><td>{item(m.id)}</td><td>{n(m.count)}</td><td>{m.owned===null?'неизвестно':n(m.owned)}</td><td>{m.allocated===null?'неизвестно':n(m.allocated)}</td><td>{n(m.crafted??0)}</td><td>{m.missing===null?'неизвестно':n(m.missing)}</td></tr>)}</tbody></table></div>
      </details>)}
      {allocation.craftInputs.length>0&&<details className="craft-inputs"><summary>Сырьё выделено на крафт</summary><ul>{allocation.craftInputs.map(m=><li key={m.id}>{item(m.id)} × {n(m.count)} · в запасе {m.owned===null?'неизвестно':n(m.owned)}</li>)}</ul><p>Это сырьё уже исключено из запаса для остальных целей.</p></details>}
      <div className="wish-pity-grid planner-goals">{plans.map(({goal:g,plan:p,materials,mora,ready,crafts,craftMora,linkedFrom},index)=><article className="wish-pity-card" key={g.goal_id} tabIndex={-1}
          ref={node=>{if(node)goalCards.current.set(g.goal_id,node);else goalCards.current.delete(g.goal_id);}}>
        <small>Очередь {index+1} · Приоритет {linkedFrom?Math.min(g.values.priority,linkedFrom.values.priority):g.values.priority}</small>
        <h3>{p.values?name(p.values):linkedFrom?'Экипированное оружие':'Цель'}</h3>
        {linkedFrom&&<p>Оружие цели «{name(choices.find(c=>matches(linkedFrom,c))?.values??{})}»</p>}<p>{p.complete?'Цель достигнута':p.status==='unknown'?p.reason:`Цель: ур. ${p.targetLevel}, возвышение ${p.targetAsc}`}</p>
        {p.status==='saved'&&<p>Расчёт по сохранённым данным.</p>}
        {p.requiredAdventureRank>0&&(p.currentRank===null||p.currentRank<p.requiredAdventureRank)&&<p>Для выполнения нужен ранг приключений {p.requiredAdventureRank}.</p>}
        {p.status!=='unknown'&&!p.complete&&<>
          <p className={`upgrade-status ${ready?'ready':'shortage'}`}>{ready?(crafts.length?'Можно выполнить после крафта':'Все материалы выделены'):materials.some(m=>m.missing===null)||mora!.missing===null?'Запасы не подтверждены':'Не все материалы выделены'}</p>
          <p>{p.experience>0&&<>Осталось опыта: {n(p.experience)}. </>}Мора: {n(mora!.count)}; выделено {mora!.allocated===null?'неизвестно':n(mora!.allocated)}; не хватает {mora!.missing===null?'неизвестно':n(mora!.missing)}.</p>
          <div className="table-wrap"><table><thead><tr><th>Ресурс</th><th>Нужно</th><th>Из запаса</th><th>Крафт</th><th>Не хватает</th></tr></thead><tbody>{materials.filter(visibleMaterial).map(m=><tr key={m.id}><td>{item(m.id)}{m.uncertain&&' · запас опыта не подтверждён'}</td><td>{n(m.count)}</td><td>{m.allocated===null?'неизвестно':n(m.allocated)}</td><td>{n(m.crafted??0)}</td><td>{m.missing===null?'неизвестно':n(m.missing)}</td></tr>)}</tbody></table></div>
          {crafts.length>0&&<details className="craft-steps"><summary>Крафт: {n(craftMora)} моры</summary><ol>{crafts.map((step,i)=><li key={`${step.recipeId}:${i}`}>{item(step.inputId)} × {n(step.inputCount)} → {item(step.itemId)} × {n(step.count)}<br/><small>{n(step.mora)} моры</small></li>)}</ol><p>Стоимость крафта включена в общую мору цели. Предметы появятся в инвентаре после крафта в игре.</p></details>}
        </>}
        {g.goal_id.endsWith(':weapon')
          ? <button disabled={busy} onClick={()=>setDraft({...linkedFrom})}>Изменить цель персонажа</button>
          : <><button disabled={busy} onClick={()=>setDraft({...g})}>Изменить</button> <button disabled={busy} onClick={()=>void save({goal_id:g.goal_id,revision:g.revision,delete:true})}>Удалить</button></>}
      </article>)}</div>
    </>}
  </section>;
}
