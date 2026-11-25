import { tr, uiLocale } from "./i18n.mjs";
import React,{useEffect,useMemo,useState} from 'react';
import {get, type Row} from './api';
import { ItemIcon } from './InventoryViews';
import { LevelTarget } from './LevelTarget';
import { equippedWeapon, maximumWeaponLevel } from './goal-targets.mjs';
import { GoalPicker, type GoalChoice } from './GoalPicker';
import {groupPlannerResources} from './planner-resources.mjs';
import {allocateGoals} from './allocation.mjs';
import {progressionReference} from './progression.mjs';
import {plannerGoalsUrl, plannerPresetName, plannerPresetStorageKey, plannerPresetsUrl, type PlannerPreset} from './planner-presets';
const n=(v:number)=>v.toLocaleString(uiLocale);
type PlannerProps = {profile:string;tick:number;data:any;catalog:any;requestedGoal?:GoalChoice;onRequestHandled:()=>void;openMaterial:(id:number)=>void;openEntity:(row:Row)=>void};

export function Planner(props: PlannerProps) {
  const storageKey = plannerPresetStorageKey(props.profile);
  const [presets, setPresets] = useState<PlannerPreset[]>([]);
  const [selected, setSelected] = useState(() => localStorage.getItem(storageKey) ?? "default");
  const [editor, setEditor] = useState<{id?:string;name:string} | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const endpoint = plannerPresetsUrl(props.profile);
  const active = presets.find(preset => preset.id === selected) ?? presets.find(preset => preset.id === "default");
  useEffect(() => {
    const controller = new AbortController();
    get<{presets:PlannerPreset[]}>(endpoint, controller.signal)
      .then(data => setPresets(data.presets))
      .catch(error => { if (!controller.signal.aborted) setError(error.message); });
    return () => controller.abort();
  }, [endpoint]);

  function choose(id:string) {
    setEditor(null);
    setSelected(id);
    localStorage.setItem(storageKey, id);
  }

  async function savePreset() {
    if (!editor || busy) return;
    setBusy(true);
    setError("");
    try {
      const response = await fetch(endpoint, {method:"POST",
        headers:{"Content-Type":"application/json","X-Vision-Control":"1"},
        body:JSON.stringify(editor)});
      const result = await response.json();
      if (!response.ok) throw Error(result.error?.message ?? tr("Не удалось сохранить пресет."));
      const preset: PlannerPreset = result;
      setPresets(previous => [...previous.filter(item => item.id !== preset.id), preset]);
      choose(preset.id);
    } catch (error) {
      setError(error instanceof Error ? error.message : tr("Не удалось сохранить пресет."));
    } finally {setBusy(false);}
  }

  return <>
    <div className="planner-presets">
      <label>{tr("Пресет ")}<select value={active?.id ?? ""} disabled={!active || busy} onChange={e=>choose(e.target.value)}>
        {presets.map(preset => <option key={preset.id} value={preset.id}>{plannerPresetName(preset)}</option>)}
      </select></label>
      <button type="button" disabled={!active || busy} onClick={()=>setEditor({name:""})}>{tr("Новый пресет")}</button>
      {active && <button type="button" disabled={busy} onClick={()=>setEditor(active)}>{tr("Переименовать")}</button>}
    </div>
    {error && <p role="alert">{error}</p>}
    {editor && <form className="planner-presets" onSubmit={e=>{e.preventDefault();void savePreset();}}>
      <label>{tr("Название пресета ")}<input autoFocus required maxLength={64} value={editor.name}
        onChange={e=>setEditor({...editor,name:e.target.value})} /></label>
      <button disabled={busy || !editor.name.trim()}>{tr("Сохранить")}</button>
      <button type="button" disabled={busy} onClick={()=>setEditor(null)}>{tr("Отмена")}</button>
    </form>}
    {active && <PresetGoals key={active.id} {...props} preset={active.id} />}
  </>;
}

function PresetGoals({profile,tick,data,catalog,requestedGoal,onRequestHandled,preset,openMaterial,openEntity}:PlannerProps & {preset:string}) {
  const [goals,setGoals]=useState<any[]>([]),[draft,setDraft]=useState<any>(null),[error,setError]=useState(''),[reload,setReload]=useState(0),[busy,setBusy]=useState(false);
  const [pickerOpen,setPickerOpen]=useState(false),[hideCollected,setHideCollected]=useState(false),[loaded,setLoaded]=useState(false);
  const [focusedGoal,setFocusedGoal]=useState<string|null>(null);
  const goalCards=React.useRef(new Map<string,HTMLElement>());
  const draftForm=React.useRef<HTMLFormElement>(null);
  const matches=(goal:any, choice:GoalChoice)=>goal.values.collection===choice.collection&&goal.values.entity_key===choice.entity_key;
  const visibleMaterial=(m:{missing:number|null;crafted?:number})=>!hideCollected||m.missing!==0||Boolean(m.crafted);
  useEffect(()=>{const c=new AbortController();get(plannerGoalsUrl(profile,preset),c.signal).then(v=>{setGoals(v.goals);setLoaded(true);}).catch(e=>{if(!c.signal.aborted)setError(e.message);});return ()=>c.abort();},[profile,preset,tick,reload]);
  const choices=useMemo<GoalChoice[]>(()=>(['characters','weapons'] as const).flatMap(collection=>(data.collections[collection]??[]).filter((r:any)=>!r.values.trial_avatar_info&&r.values.avatar_id!==10000134).map((r:any)=>({collection,...r}))),[data]);
  const name=(v:any)=>v.avatar_id?(catalog.avatarNames[v.avatar_id]??tr("Персонаж {0}", v.avatar_id)):(catalog.names[v.item_id]??tr("Оружие {0}", v.item_id));
  const allocation=useMemo(()=>allocateGoals(goals,catalog,data),[goals,catalog,data]);
  const plans=allocation.plans;
  const linkedWeaponKeys = useMemo(() => new Set(goals
    .filter(goal => goal.values.collection === "characters" && goal.values.equipped_weapon)
    .map(goal => equippedWeapon(data,goal.values.entity_key)?.entity_key)),[data,goals]);
  function item(id:number) {
    const label = catalog.names[id] ?? tr("Предмет {0}", id);
    return <button className="planner-item-link" type="button" onClick={()=>openMaterial(id)}>
      <ItemIcon icon={catalog.items?.[id]?.icon} rarity={catalog.rarities[id]??catalog.items?.[id]?.rarity} />
      <span>{label}</span>
    </button>;
  }
  async function save(body:any) {
    setBusy(true);setError('');
    try {const r=await fetch(`/api/v1/profiles/${profile}/goals`,{method:'POST',headers:{'Content-Type':'application/json','X-Vision-Control':'1'},body:JSON.stringify({...body,preset_id:preset})});const v=await r.json();if(!r.ok)throw Error(v.error?.message??tr("Не удалось сохранить цель."));setDraft(null);setLoaded(false);setReload(x=>x+1);}
    catch(e){setError(e instanceof Error?e.message:tr("Ошибка сохранения."));}finally{setBusy(false);}
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
    else setError(tr("Цель больше не доступна в текущем профиле."));
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
    <button className="planner-picker-button" type="button" disabled={!loaded||busy} onClick={()=>setPickerOpen(true)}>{draft?tr("Изменить цель"):tr("Добавить цель")}</button>
    {pickerOpen&&<GoalPicker choices={choices.filter(choice=>!goals.some(goal=>matches(goal,choice)) &&
        (choice.collection!=="weapons" || !linkedWeaponKeys.has(choice.entity_key)))}
      catalog={catalog} characters={data.collections.characters??[]} name={name}
      onClose={()=>setPickerOpen(false)} onSelect={choice=>{setPickerOpen(false);select(choice);}} />}
    {draft&&<form ref={draftForm} className="wish-pity-card" onSubmit={e=>{e.preventDefault();void save({goal_id:draft.goal_id,revision:draft.revision,values:draft.values});}}>
      <h3>{tr("Цели прокачки")}</h3>
      {draftRow&&<h4>{name(draftRow.values)}</h4>}
      <div className="goal-parameters">
        {characterDraft&&<section>
          <h4>{tr("Возвышение персонажа")}</h4>
          <LevelTarget value={draft.values} onChange={change} />
        </section>}
        {characterDraft&&Object.keys(draft.values.talents).length>0&&<section>
          <h4>{tr("Таланты")}</h4>
          {Object.entries(draft.values.talents).map(([id,level])=><label className="goal-talent" key={id}>
            {catalog.skillNames[id]??tr("Талант {0}", id)}
            <input aria-label={tr("Целевой базовый уровень: {0}", catalog.skillNames[id]??id)} type="number" min="1" max="10"
              value={Number(level)} onChange={e=>change({talents:{...draft.values.talents,[id]:e.target.valueAsNumber}})} />
          </label>)}
          <small>{tr("Требования талантов могут повысить расчётный уровень и возвышение.")}</small>
        </section>}
        <section>
          <h4>{tr("Оружие")}</h4>
          {characterDraft&&<label className="planner-hide-collected">
            <input type="checkbox" checked={!!draft.values.equipped_weapon} onChange={e=>toggleWeapon(e.target.checked)} />{tr(" Экипированное оружие")}</label>}
          {characterDraft&&draft.values.equipped_weapon&&<>
            <small>{weapon?name(weapon.values):tr("Экипированное оружие ещё не определено.")}</small>
            <LevelTarget value={draft.values.equipped_weapon} onChange={target=>change({equipped_weapon:target})} />
          </>}
          {!characterDraft&&<LevelTarget value={draft.values} max={maximumWeaponLevel(draftRow,catalog)} onChange={change} />}
        </section>
        <label>{tr("Приоритет (меньше — выше) ")}<input aria-label={tr("Приоритет цели")} type="number" min="0" max="999" value={draft.values.priority} onChange={e=>change({priority:Number(e.target.value)})} /></label>
      </div>

      <button disabled={busy}>{tr("Сохранить цель")}</button> <button type="button" onClick={()=>setDraft(null)}>{tr("Отмена")}</button>
    </form>}
    {!goals.length&&<p>{tr("Пока нет целей. Выберите персонажа или конкретный экземпляр оружия.")}</p>}
    {goals.length>0&&<>
      <h3>{tr("Распределение ресурсов")}</h3>
      <label className="planner-hide-collected"><input type="checkbox" checked={hideCollected} onChange={e=>setHideCollected(e.target.checked)} />{tr(" Скрыть собранное")}</label>
      <p>{tr("Сначала выделяются материалы для прямых потребностей всех целей. Остаток идёт на крафт от низкой редкости к высокой. Если материала мало, его получают цели с меньшим числом приоритета.")}</p>
      {!catalog.crafting&&<p role="status">{tr("В выбранной версии справочника нет рецептов. Крафт не включён в расчёт.")}</p>}
      {allocation.partial&&<p role="status">{tr("Часть целей не удалось рассчитать. Для них материалы не выделены.")}</p>}
      <p>{tr("Книги и руда подбираются из запасов. Крафт учитывает сырьё и ранг приключений. Его стоимость добавляется к общей потребности в море.")}</p>
      {groupPlannerResources(allocation.materials.filter(visibleMaterial),catalog).map(group=><details className="planner-resource-group" key={group.key} open>
        <summary>{tr(group.title)}</summary>
      <div className="table-wrap"><table><thead><tr><th>{tr("Ресурс")}</th><th>{tr("Нужно всего")}</th><th>{tr("Из них на крафт")}</th><th>{tr("Есть")}</th><th>{tr("Из запаса")}</th><th>{tr("Получим крафтом")}</th><th>{tr("Не хватает")}</th></tr></thead><tbody>{group.rows.map(m=><tr key={m.id}><td>{item(m.id)}</td><td>{n(m.count)}</td><td>{m.craftInput?n(m.craftInput):"—"}</td><td>{m.owned===null?tr("неизвестно"):n(m.owned)}</td><td>{m.allocated===null?tr("неизвестно"):n(m.allocated)}</td><td>{n(m.crafted??0)}</td><td>{m.missing===null?tr("неизвестно"):n(m.missing)}</td></tr>)}</tbody></table></div>
      </details>)}
      {allocation.craftInputs.length>0&&<details className="craft-inputs"><summary>{tr("Сырьё выделено на крафт")}</summary><ul>{allocation.craftInputs.map(m=><li key={m.id}>{item(m.id)} × {n(m.count)}{tr(" · в запасе ")}{m.owned===null?tr("неизвестно"):n(m.owned)}</li>)}</ul><p>{tr("Это сырьё уже исключено из запаса для остальных целей.")}</p></details>}
      <div className="wish-pity-grid planner-goals">{plans.map(({goal:g,plan:p,materials,mora,ready,crafts,craftMora,linkedFrom},index)=>{
        const row = g.goal_id.endsWith(':weapon')
          ? equippedWeapon(data,linkedFrom?.values.entity_key)
          : choices.find(choice=>matches(g,choice));
        const v = p.values ?? row?.values;
        const isCharacter = !!v?.avatar_id;
        const title = v ? name(v) : linkedFrom ? tr("Экипированное оружие") : tr("Цель");
        const icon = isCharacter ? catalog.avatarIcons?.[v.avatar_id] : catalog.items?.[v?.item_id]?.icon;
        const level = v?.level ?? v?.props?.['4001']?.ival;
        const ascension = v?.ascension ?? v?.props?.['1002']?.ival;
        const sameDepot = isCharacter && Number(v.skill_depot_id) === g.values.skill_depot_id;
        const talentIds = [...new Set([
          ...(sameDepot ? progressionReference(v,catalog)?.talents.map((talent:any)=>talent.skillId) ?? [] : []),
          ...Object.keys(g.values.talents ?? {}),
        ])];
        return <article className="wish-pity-card" key={g.goal_id} tabIndex={-1}
          ref={node=>{if(node)goalCards.current.set(g.goal_id,node);else goalCards.current.delete(g.goal_id);}}>
        <small>{tr("Очередь ")}{index+1}{tr(" · Приоритет ")}{linkedFrom?Math.min(g.values.priority,linkedFrom.values.priority):g.values.priority}</small>
        <h3>{row ? <button type="button" className="planner-item-link"
          onClick={()=>openEntity(row)}>
          <ItemIcon icon={icon} rarity={isCharacter ? 0 : catalog.rarities[v?.item_id]} fallback={title.slice(0,1)} />
          <span>{title}</span>
        </button> : title}</h3>
        {linkedFrom&&<p>{tr("Оружие цели «")}{name(choices.find(c=>matches(linkedFrom,c))?.values??{})}»</p>}
        {v && <dl className="planner-progress">
          <dt>{tr("Сейчас")}</dt><dd>{tr("Ур. ")}{level ?? '—'}{tr(" · Возвышение ")}{ascension ?? '—'}</dd>
          {!p.complete && p.status!=='unknown' && <><dt>{tr("Цель")}</dt><dd>{tr("Ур. ")}{p.targetLevel}{tr(" · Возвышение ")}{p.targetAsc}</dd></>}
        </dl>}
        {talentIds.length > 0 && <div className="planner-talent-progress">
          <strong>{tr("Базовые уровни талантов")}</strong>
          <dl className="planner-progress">{talentIds.map(id=>{
            const current = sameDepot ? v.skill_levels?.[id] : undefined;
            const target = g.values.talents?.[id];
            return <React.Fragment key={id}>
              <dt>{catalog.skillNames[id]??tr("Талант {0}", id)}</dt>
              <dd>{current ?? '—'}{target != null && (current == null || Number(current) < target) && ` → ${target}`}</dd>
            </React.Fragment>;
          })}</dl>
        </div>}
        {(p.complete || p.status==='unknown') && <p>{p.complete?tr("Цель достигнута"):p.reason}</p>}
        {p.status==='saved'&&<p>{tr("Расчёт по сохранённым данным.")}</p>}
        {p.requiredAdventureRank>0&&(p.currentRank===null||p.currentRank<p.requiredAdventureRank)&&<p>{tr("Для выполнения нужен ранг приключений ")}{p.requiredAdventureRank}.</p>}
        {p.status!=='unknown'&&!p.complete&&<>
          <p className={`upgrade-status ${ready?'ready':'shortage'}`}>{ready?(crafts.length?tr("Можно выполнить после крафта"):tr("Все материалы выделены")):materials.some(m=>m.missing===null)||mora!.missing===null?tr("Запасы не подтверждены"):tr("Не все материалы выделены")}</p>
          <p>{p.experience>0&&<>{tr("Осталось опыта: ")}{n(p.experience)}. </>}{item(202)} {n(mora!.count)}{tr("; выделено ")}{mora!.allocated===null?tr("неизвестно"):n(mora!.allocated)}{tr("; не хватает ")}{mora!.missing===null?tr("неизвестно"):n(mora!.missing)}.</p>
          <div className="table-wrap"><table><thead><tr><th>{tr("Ресурс")}</th><th>{tr("Нужно")}</th><th>{tr("Из запаса")}</th><th>{tr("Крафт")}</th><th>{tr("Не хватает")}</th></tr></thead><tbody>{materials.filter(visibleMaterial).map(m=><tr key={m.id}><td>{item(m.id)}{m.uncertain&&tr(" · запас опыта не подтверждён")}</td><td>{n(m.count)}</td><td>{m.allocated===null?tr("неизвестно"):n(m.allocated)}</td><td>{n(m.crafted??0)}</td><td>{m.missing===null?tr("неизвестно"):n(m.missing)}</td></tr>)}</tbody></table></div>
          {crafts.length>0&&<details className="craft-steps"><summary>{tr("Крафт: ")}{n(craftMora)}{tr(" моры")}</summary><ol>{crafts.map((step,i)=><li key={`${step.recipeId}:${i}`}>{item(step.inputId)} × {n(step.inputCount)} → {item(step.itemId)} × {n(step.count)}<br/><small>{n(step.mora)}{tr(" моры")}</small></li>)}</ol><p>{tr("Стоимость крафта включена в общую мору цели. Предметы появятся в инвентаре после крафта в игре.")}</p></details>}
        </>}
        {g.goal_id.endsWith(':weapon')
          ? <button disabled={busy} onClick={()=>setDraft({...linkedFrom})}>{tr("Изменить цель персонажа")}</button>
          : <><button disabled={busy} onClick={()=>setDraft({...g})}>{tr("Изменить")}</button> <button disabled={busy} onClick={()=>void save({goal_id:g.goal_id,revision:g.revision,delete:true})}>{tr("Удалить")}</button></>}
      </article>;})}</div>
    </>}
  </section>;
}
