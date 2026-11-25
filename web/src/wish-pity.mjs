import { tr } from "./i18n.mjs";
import {compareWishes} from "./wish-live.mjs";
// Only imported records and continuity-checked capture observations participate.
export const wishGroups = [
  {id:'characters',name:tr("События персонажей I + II"),types:['301','400'],cap:90},
  {id:'weapons',name:tr("Событие оружия"),types:['302'],cap:80},
  {id:'standard',name:tr("Стандартная молитва"),types:['200'],cap:90},
  {id:'chronicled',name:tr("Молитва хроник"),types:['500'],cap:90},
  {id:'beginners',name:tr("Молитва новичка"),types:['100'],cap:null},
];
const cmp=(a,b)=>a.length-b.length||a.localeCompare(b);
const normalize=s=>String(s??'').normalize('NFKC').trim().replace(/ё/g,'е').toLocaleLowerCase('ru');
function localTime(s) {
  if(!/^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d$/.test(s??''))return null;
  return Date.parse(s.replace(' ','T')+'Z'); // A wall clock scalar, independent of the browser timezone.
}
export function classifyFeatured(row, group, catalog, periods, zone) {
  if(!['characters','weapons'].includes(group))return null;
  if(!Number.isInteger(zone)) {
    // Missing API timezone is safe only when every valid offset gives the same result.
    const outcomes=Array.from({length:27},(_,i)=>classifyFeatured(row,group,catalog,periods,i-12));
    return outcomes.every(v=>v===outcomes[0])?outcomes[0]:null;
  }

  const time=localTime(row.time);
  if(time===null)return null;
  const matches=periods.filter(p=>{
    if(p.group!==group)return false;
    if((p.startGlobal||p.endGlobal)&&!Number.isInteger(zone))return false;
    const start=localTime(p.start)+(p.startGlobal?(zone-8)*3600000:0);
    const end=localTime(p.end)+(p.endGlobal?(zone-8)*3600000:0);
    return time>=start&&time<=end;
  });
  if(matches.length!==1)return null; // Missing or overlapping metadata cannot prove a loss.
  const definitions=group==='characters'?catalog.avatarNames:catalog.names;
  const id=String(row.item_id??'');
  let name=id&&definitions?.[id];
  if(!name) {
    if(row.lang&&!String(row.lang).startsWith('ru'))return null;
    const ids=Object.keys(definitions??{}).filter(k=>normalize(definitions[k])===normalize(row.name));
    if(ids.length!==1)return null;
    name=definitions[ids[0]];
  }
  const featured=matches[0].featured;
  if(!featured.length||featured.some(r=>!r.name))return null;
  return featured.some(r=>normalize(r.name)===normalize(name));
}
/** @param {any[]} records @param {{catalog?:any,periods?:any[],coverage?:any,zone?:number|null}} options */
export function analyzeWishes(records, {catalog={},periods=[],coverage={},zone=null}={}) {
  const unique=new Map();let duplicate=false;
  for(const r of records) {if(unique.has(r.id))duplicate=true;unique.set(r.id,r);}
  const sorted=[...unique.values()].sort(compareWishes);
  return wishGroups.map(group=>{
    const rows=sorted.filter(r=>group.types.includes(r.gacha_type));
    const five=rows.filter(r=>r.rank_type==='5');
    const last=five.at(-1);const at=last?rows.findIndex(r=>r.id===last.id):-1;
    const pity=rows.length-at-1;
    const range=coverage.ranges?.[group.id];
    const exact=!!last&&!duplicate&&!rows.some(r=>!["3","4","5"].includes(r.rank_type))&&coverage.all_available_pages_fetched===true&&(last.live===true || (!!range&&cmp(range.first,last.id)<=0&&cmp(last.id,range.last)<=0));
    const invalid=!!group.cap&&pity>=group.cap;
    let guarantee='not_applicable',reason='';
    if(['characters','weapons'].includes(group.id)) {
      guarantee='unknown';reason=tr("Для гарантии нужен последний 5★ и подтверждённый состав баннера на дату выпадения.");
      if(last&&exact&&!invalid) {
        const featured=classifyFeatured(last,group.id,catalog,periods,zone);
        if(featured===false){guarantee='guaranteed';reason=tr("Последний 5★ не был акционным; следующий 5★ — акционный.");}
        if(featured===true){guarantee='not_guaranteed';reason=tr("Последний 5★ был акционным; событийный гарант сейчас не подтверждён.");}
      }else if(last&&!exact)reason=tr("Последний 5★ не входит в подтверждённый диапазон последней выгрузки; возможны пропуски.");
    }else if(group.id==='chronicled') {
      guarantee='unknown';reason=tr("История не содержит выбранный курс и его изменения. Очки судьбы не восстанавливаются по одним выпадениям.");
    }
    let previous=-1;
    const drops=five.map(r=>{
      const index=rows.findIndex(x=>x.id===r.id),count=index-previous;
      const result={id:r.id,count,exact:previous>=0&&coverage.all_available_pages_fetched===true&&(rows[previous].live===true || (!!range&&cmp(range.first,rows[previous].id)<=0))&&(r.live===true || (!!range&&cmp(r.id,range.last)<=0))};previous=index;return result;
    });
    return {...group,total:rows.length,five:five.length,last,pity,exact:exact&&!invalid,invalid,
      remaining:exact&&!invalid&&group.cap?group.cap-pity:null,guarantee,reason,drops,
      path:group.id==='weapons'?'unknown':null};
  });
}
