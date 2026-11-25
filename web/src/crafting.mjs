/** Unique acyclic 3:1 tier paths supplied by the validated catalog. Each attempt
 * uses a scratch ledger so a failed batch leaves every ingredient available.
 */
export function craftFromStock(id, wanted, recipes, rank, remaining, commit) {
  const attempt=count=>{
    const ledger=new Map(),inputs=new Map(),steps=[];
    const left=id=>{if(!ledger.has(id))ledger.set(id,remaining(id));return ledger.get(id);};
    function ensure(id,need,path=new Set()) {
      if(!Number.isSafeInteger(need)||need<0)return false;
      if(!need)return true;
      const existing=Math.min(left(id)??0,need);
      if(existing) {ledger.set(id,left(id)-existing);inputs.set(id,(inputs.get(id)??0)+existing);need-=existing;}
      if(!need)return true;
      const r=recipes?.[id];
      if(!r||path.has(id)||path.size>=16||!Number.isInteger(rank)||rank<r.requiredAdventureRank)return false;
      const visited=new Set(path);visited.add(id);
      if(!ensure(r.inputId,need*r.inputCount,visited))return false;
      const cost=need*r.mora;
      if(!Number.isSafeInteger(cost))return false;
      steps.push({recipeId:r.recipeId,itemId:id,count:need,inputId:r.inputId,inputCount:need*r.inputCount,mora:cost});
      return true;
    }
    return ensure(id,count)?{ledger,inputs,steps}:null;
  };
  let low=0,high=wanted,best=null;
  while(low<high) {const mid=Math.ceil((low+high)/2),result=attempt(mid);if(result){low=mid;best=result;}else high=mid-1;}
  if(!low)return {count:0,inputs:[],steps:[],mora:0};
  best ??= attempt(low);
  for(const [id,left] of best.ledger)commit(id,left);
  return {count:low,inputs:[...best.inputs].map(([id,count])=>({id,count})),steps:best.steps,mora:best.steps.reduce((n,s)=>n+s.mora,0)};
}
