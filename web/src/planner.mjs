import { tr } from "./i18n.mjs";
import {progressionReference} from './progression.mjs';
import {expMaterials,observedStock} from './upgrades.mjs';
const prop=(v,k,id)=>Number(v[k]??v.props?.[id]?.ival??v.props?.[id]?.val);
const valid=n=>Number.isSafeInteger(n)&&n>=0;

/** Remaining costs are recomputed from game facts; user targets are never mutated. */
export function planGoal(goal,catalog,data) {
  const g=goal.values??goal;
  const row=data.collections?.[g.collection]?.find(r=>r.entity_key===g.entity_key);
  const unknown=reason=>({status:'unknown',reason,materials:[],mora:0,experience:0,requiredAdventureRank:0,currentRank:null,complete:false,values:row?.values,targetLevel:null,targetAsc:null});
  if(!row)return unknown(tr("Данные выбранного предмета или персонажа отсутствуют."));
  const character=g.collection==='characters';let v={...row.values};let cached=false;
  if(character&&Number(v.skill_depot_id)!==g.skill_depot_id) {
    const saved=data.collections?.traveler_depots?.find(r=>Number(r.entity_key)===g.skill_depot_id&&r.values.guid===g.entity_key);
    if(!saved)return unknown(tr("Набор талантов этой стихии ещё не получен."));
    v={...v,...saved.values};cached=saved.confirmation!=='observed';
  }
  const ref=progressionReference(v,catalog);if(!ref)return unknown(tr("Путь прокачки ещё не сопоставлен со справочником."));
  let level=character?prop(v,'level','4001'):Number(v.level);
  let exp=character?prop(v,'exp','1001'):Number(v.exp);
  const asc=character?prop(v,'ascension','1002'):Number(v.ascension);
  if(![level,exp,asc].every(valid)||level<1)return unknown(tr("Уровень, опыт или возвышение не подтверждены."));
  const p=catalog.progression;
  const pathId=(character?p.avatarPaths:p.weaponPaths)[String(character?v.avatar_id:v.item_id)];
  const path=(character?p.avatarPromotions:p.weaponPromotions)[String(pathId)];
  if(!path[String(asc)]||level>path[String(asc)].maxLevel)return unknown(tr("Уровень не соответствует возвышению."));
  const last=Object.values(path).at(-1); // Paths are validated contiguous numeric stages.
  let targetLevel=Math.max(level,g.level),targetAsc=Math.max(asc,g.ascension);
  const talentCosts=[];
  for(const [skill,target] of Object.entries(g.talents??{})) {
    const base=Number(v.skill_levels?.[skill]);
    const skillRef=ref.talents.find(t=>t.skillId===skill);
    if(!valid(base)||base<1||!skillRef)return unknown(tr("Базовые уровни выбранных талантов неизвестны."));
    for(let n=base+1;n<=target;n++) {
      const step=skillRef.levels.find(s=>s.level===n);
      if(!step||step.requiredLevel===null)return unknown(tr("Стоимость или требования таланта отсутствуют."));
      talentCosts.push(step);targetAsc=Math.max(targetAsc,step.requiredAscension);targetLevel=Math.max(targetLevel,step.requiredLevel);
    }
  }
  while(path[String(targetAsc)]&&path[String(targetAsc)].maxLevel<targetLevel)targetAsc++;
  if(!path[String(targetAsc)]||targetLevel>last.maxLevel)return unknown(tr("Цель выходит за доступный путь прокачки."));
  targetLevel=Math.max(targetLevel,path[String(targetAsc)].requiredLevel);
  const materials=new Map();let mora=0,experience=0;
  const add=(id,count)=>materials.set(Number(id),(materials.get(Number(id))??0)+count);
  const addStep=step=>{if(step.materialsAvailable===false)throw Error(tr("Материалы этапа не сопоставлены."));mora+=step.mora;for(const m of step.materials??[])add(m.id,m.count);};
  try {
    for(let stage=asc+1;stage<=targetAsc;stage++)addStep(path[String(stage)]);
    for(const step of talentCosts)addStep(step);
  }catch(e){return unknown(e.message);}
  const fixedMaterials=[...materials].map(([id,count])=>({id,count})),fixedMora=mora,experienceSegments=[];
  const leveling=catalog.leveling,rarity=catalog.rarities?.[String(v.item_id)];
  const exps=character?leveling?.avatarExp:leveling?.weaponExp?.[String(rarity)];
  const defs=character?leveling?.avatarMaterials:leveling?.weaponMaterials;
  const ratio=character?leveling?.avatarMoraPerExp:leveling?.weaponMoraPerExp?.[String(rarity)];
  // Plan each ascension segment separately: EXP overflow cannot cross a cap.
  if(level<targetLevel&&(!exps||!defs||!ratio))return unknown(tr("Таблица опыта недоступна."));
  for(let stage=asc;stage<=targetAsc&&level<targetLevel;stage++) {
    const cap=path[String(stage)].maxLevel,end=Math.min(cap,targetLevel);
    if(level>end)return unknown(tr("Уровень не соответствует этапу возвышения."));
    let required=0;
    for(let n=level;n<end;n++){if(!exps[String(n)])return unknown(tr("В таблице опыта есть пропуск."));required+=exps[String(n)];}
    if(level<end) {
      required-=exp;if(required<=0)return unknown(tr("Текущий опыт не соответствует уровню."));
      experienceSegments.push({required,definitions:defs,ratio});
      const selection=expMaterials(required,defs,()=>0); // Canonical requirement; distribution is a separate step.
      if(!selection)return unknown(tr("Не удалось рассчитать материалы опыта."));
      for(const m of selection.materials)add(m.id,m.count);
      mora+=Math.ceil(selection.suppliedExp*ratio.numerator/ratio.denominator);
      experience+=required;level=end;exp=0;
    }
  }
  const complete=materials.size===0&&mora===0;
  const costs=[...materials].map(([id,count])=>({id,count,owned:observedStock(data,id)}));
  const requiredAdventureRank=Math.max(0,...Array.from({length:Math.max(0,targetAsc-asc)},(_,i)=>path[String(asc+i+1)].requiredAdventureRank));
  const currentRank=Number(data.collections?.adventure?.[0]?.values.rank);
  const fresh=row.confirmation==='observed'&&!cached&&[g.collection,'material_stacks','wallet'].every(c=>data.domains?.find(d=>d.collection===c)?.freshness==='synchronized');
  return {status:complete?'complete':fresh?'active':'saved',complete,materials:costs,mora,experience,targetLevel,targetAsc,
    fixedMaterials,fixedMora,experienceSegments,moraOwned:observedStock(data,202,true),values:v,requiredAdventureRank,currentRank:valid(currentRank)?currentRank:null,reason:null};
}

export function summarizePlans(plans,data) {
  const totals=new Map();let mora=0;
  for(const p of plans)if(p.status!=='unknown') {mora+=p.mora;for(const m of p.materials)totals.set(m.id,(totals.get(m.id)??0)+m.count);}
  return {partial:plans.some(p=>p.status==='unknown'),mora,moraOwned:observedStock(data,202,true),
    materials:[...totals].map(([id,count])=>{const owned=observedStock(data,id);return {id,count,owned,missing:owned===null?null:Math.max(0,count-owned)};})};
}
