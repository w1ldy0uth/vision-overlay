import {craftFromStock} from './crafting.mjs';
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
  const ordered=[...goals].sort((a,b)=>a.values.priority-b.values.priority||a.goal_id.localeCompare(b.goal_id));
  const plans=ordered.map(goal=>{
    const plan=planGoal(goal,catalog,data);
    if(plan.status==='unknown')return {goal,plan,materials:[],craftInputs:[],crafts:[],craftMora:0,mora:null,ready:false,experienceMissing:null};
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
      if(!selected)throw Error('Не удалось распределить материалы опыта.');
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
    // Reserve the goal's upgrade cost first; crafting can use only the excess
    // wallet. Every higher-priority goal finishes before the next one starts.
    const moraAllocated=take(202,mora);
    const crafts=[],craftInputs=new Map();let craftMora=0;
    for(const m of materials)if(m.missing!==null&&m.missing>0) {
      const crafted=craftFromStock(m.id,m.missing,catalog.crafting?.recipes,plan.currentRank,remaining,(id,left)=>available.set(id,left));
      m.crafted=crafted.count;m.missing-=crafted.count;craftMora+=crafted.mora;crafts.push(...crafted.steps);
      for(const input of crafted.inputs)craftInputs.set(input.id,(craftInputs.get(input.id)??0)+input.count);
    }
    const money={id:202,count:mora+craftMora,owned:owned(202),allocated:moraAllocated===null?null:moraAllocated+craftMora,missing:moraAllocated===null?null:mora-moraAllocated};
    return {goal,plan,materials,mora:money,crafts,craftMora,craftInputs:[...craftInputs].map(([id,count])=>({id,count,owned:owned(id)})),experienceMissing:unknownExperience?null:experienceMissing,
      ready:materials.every(m=>m.missing===0)&&money.missing===0};
  });
  const totals=new Map();
  for(const result of plans)for(const m of [...result.materials,...(result.mora?[result.mora]:[])]) {
    if(!m.count)continue;
    const row=totals.get(m.id)??{id:m.id,count:0,owned:m.owned,allocated:0,crafted:0,missing:0};
    row.count+=m.count;row.crafted+=m.crafted??0;
    row.allocated=row.allocated===null||m.allocated===null?null:row.allocated+m.allocated;
    row.missing=row.missing===null||m.missing===null?null:row.missing+m.missing;
    totals.set(m.id,row);
  }
  const raw=new Map();
  for(const p of plans)for(const input of p.craftInputs){const r=raw.get(input.id)??{...input,count:0};r.count+=input.count;raw.set(input.id,r);}
  return {plans,materials:[...totals.values()],craftInputs:[...raw.values()],partial:plans.some(p=>p.plan.status==='unknown')};
}
