import { tr } from "./i18n.mjs";
import {craftFromStock} from './crafting.mjs';
import {resolveGoalTargets} from './goal-targets.mjs';
import {planGoal} from './planner.mjs';
import {expMaterials, observedStock} from './upgrades.mjs';

/** Virtual reservations: the same observed item can belong to only one goal.
 * No inventory, goal or catalog object is changed. Rebuilding releases completed
 * goals and consumes external spending before priorities are applied again.
 */
export function allocateGoals(goals, catalog, data) {
  const available=new Map(), stock=new Map();
  const owned=id=>{
    if(!stock.has(id)) {
      const value=observedStock(data,id,id===202);
      stock.set(id,value);available.set(id,value);
    }
    return stock.get(id);
  };
  const remaining=id=>{owned(id);return available.get(id);};
  const take=(id,count)=>{
    const left=remaining(id);
    if(left===null)return null;
    const reserved=Math.min(left,count);
    available.set(id,left-reserved);return reserved;
  };
  const ordered=resolveGoalTargets(goals,catalog,data).sort((a,b)=>a.target.priority-b.target.priority||a.goal.goal_id.localeCompare(b.goal.goal_id));
  const plans=ordered.map(({goal,target,linkedFrom})=>{
    const plan=planGoal(target,catalog,data);
    if(linkedFrom&&!target.entity_key)plan.reason=tr("Экипированное оружие ещё не определено.");
    if(plan.status==='unknown')return {goal,plan,...(linkedFrom?{linkedFrom}:{}),materials:[],crafts:[],craftMora:0,mora:null,ready:false,experienceMissing:null};
    const needs=new Map();let mora=plan.fixedMora,experienceMissing=0,unknownExperience=false;
    const add=(id,count,allocated,uncertain=false)=>{
      if(!count)return;
      const row=needs.get(id)??{id,count:0,owned:owned(id),allocated:0,uncertain:false};
      row.count+=count;
      row.allocated=row.allocated===null||allocated===null?null:row.allocated+allocated;
      row.uncertain ||= uncertain;
      needs.set(id,row);
    };
    for(const m of plan.fixedMaterials)add(m.id,m.count,take(m.id,m.count));
    for(const segment of plan.experienceSegments) {
      const {required,definitions,ratio}=segment;
      const ids=Object.keys(definitions).map(Number);
      const uncertain=ids.some(id=>remaining(id)===null);
      const selected=expMaterials(required,definitions,id=>remaining(id));
      if(!selected)throw Error(tr("Не удалось распределить материалы опыта."));
      let supplied=0;
      if(selected.fromStock) {
        for(const m of selected.materials) {
          add(m.id,m.count,take(m.id,m.count));supplied+=m.count*definitions[m.id];
        }
      } else {
        unknownExperience ||= uncertain;
        // The bounded solver proved known stocks insufficient. Reserve that
        // partial supply, then express the outstanding EXP as a canonical set.
        for(const id of ids) {
          const count=remaining(id);
          if(count!==null&&count>0) {add(id,count,take(id,count));supplied+=count*definitions[id];}
        }
        const missing=Math.max(0,required-supplied);
        experienceMissing+=missing;
        const future=expMaterials(missing,definitions,()=>0);
        for(const m of future?.materials??[]) {
          add(m.id,m.count,0,uncertain);supplied+=m.count*definitions[m.id];
        }
      }
      mora+=Math.ceil(supplied*ratio.numerator/ratio.denominator);
    }
    const materials=[...needs.values()].map(m=>({...m,crafted:0,missing:m.allocated===null||m.uncertain?null:m.count-m.allocated}));
    const moraAllocated=take(202,mora);
    const money={id:202,count:mora,owned:owned(202),allocated:moraAllocated,missing:moraAllocated===null?null:mora-moraAllocated};
    /** @type {ReturnType<typeof craftFromStock>['steps']} */
    const crafts=[];
    return {goal,plan,...(linkedFrom?{linkedFrom}:{}),materials,mora:money,crafts,craftMora:0,
      experienceMissing:unknownExperience?null:experienceMissing,ready:false};
  });

  // Reserve every direct requirement before using any shared stock for crafting.
  const shortages=new Map(), unknownDemand=new Set();
  for(const result of plans)for(const material of result.materials) {
    if(material.missing===null)unknownDemand.add(material.id);
    if(material.missing>0) {
      if(!shortages.has(material.id))shortages.set(material.id,[]);
      shortages.get(material.id).push({result,material});
    }
  }
  const recipes=Object.fromEntries(Object.entries(catalog.crafting?.recipes??{})
    .filter(([id])=>!unknownDemand.has(Number(id))));
  const depths=new Map();
  const depth=id=>{
    if(!recipes[id])return 0;
    if(!depths.has(id))depths.set(id,1+depth(recipes[id].inputId));
    return depths.get(id);
  };
  const raw=new Map();
  const craftable=[...shortages.keys()].filter(id=>recipes[id]).sort((a,b)=>depth(a)-depth(b));
  for(const id of craftable)for(const {result,material} of shortages.get(id)) {
    const crafted=craftFromStock(id,material.missing,recipes,result.plan.currentRank,remaining,(id,left)=>available.set(id,left));
    material.crafted=crafted.count;
    material.missing-=crafted.count;
    result.craftMora+=crafted.mora;
    result.crafts.push(...crafted.steps);
    for(const input of crafted.inputs) {
      const row=raw.get(input.id)??{id:input.id,count:0,owned:owned(input.id)};
      row.count+=input.count;
      raw.set(input.id,row);
    }
  }

  // The wallet only determines the final mora deficit. Upgrade costs are allocated first.
  for(const result of plans) {
    if(!result.mora)continue;
    const money=result.mora;
    money.count+=result.craftMora;
    if(money.allocated!==null) {
      money.allocated+=take(202,result.craftMora);
      money.missing=money.count-money.allocated;
    }
    result.ready=result.materials.every(m=>m.missing===0)&&money.missing===0;
  }

  const totals=new Map();
  for(const result of plans)for(const m of [...result.materials,...(result.mora?[result.mora]:[])]) {
    if(!m.count)continue;
    const row=totals.get(m.id)??{id:m.id,count:0,owned:m.owned,allocated:0,crafted:0,craftInput:0,missing:0};
    row.count+=m.count;row.crafted+=m.crafted??0;
    row.allocated=row.allocated===null||m.allocated===null?null:row.allocated+m.allocated;
    row.missing=row.missing===null||m.missing===null?null:row.missing+m.missing;
    totals.set(m.id,row);
  }
  // Ingredient reservations already consumed the shared stock. Include them in
  // the same total as direct upgrades so the shortage reconciles with inventory.
  for(const input of raw.values()) {
    const row=totals.get(input.id)??{...input,count:0,allocated:0,crafted:0,craftInput:0,missing:0};
    row.count+=input.count;
    row.craftInput+=input.count;
    if(row.allocated!==null)row.allocated+=input.count;
    totals.set(input.id,row);
  }
  return {plans,materials:[...totals.values()],craftInputs:[...raw.values()],partial:plans.some(p=>p.plan.status==='unknown')};
}
