'use strict';
const $=id=>document.getElementById(id);
let state={revision:0,status:'waiting',rows:[],history:[],history_total:0,issues:0};
let connected=false;
const number=value=>value==null?'—':value.toLocaleString('ru');
const signed=value=>`${value>=0?'+':''}${number(value)}`;
const elapsed=value=>value==null?'—':`${Math.floor(value/60000)}:${String(Math.floor(value/1000)%60).padStart(2,'0')}.${String(value%1000).padStart(3,'0')}`;
const timestamp=value=>value?value.replace('T',' '):'—';
function table(id,rows,columns,empty){
 const fragment=document.createDocumentFragment();
 for(const row of rows){const tr=document.createElement('tr');for(const value of columns(row)){const td=document.createElement('td');td.textContent=value;tr.append(td);}fragment.append(tr);}
 if(!rows.length){const tr=document.createElement('tr'),td=document.createElement('td');td.colSpan=$(id).closest('table').querySelectorAll('thead th').length;td.textContent=empty;tr.append(td);fragment.append(tr);}
 $(id).replaceChildren(fragment);
}
function status(){
 const kind=connected||state.status==='stopped'?state.status:'stale';
 const labels={waiting:'Ожидание снимка',active:'Учёт активен',stale:'Данные могут устареть',stopped:'Захват завершён',error:'Ошибка источника'};
 $('status').dataset.state=kind;
 $('status-title').textContent=labels[kind]||'Ожидание';
 $('detail').textContent=connected||state.status==='stopped'?`${state.mode==='live'?'':'Воспроизведение записи · '}${state.detail}`:'Нет соединения с локальной панелью. Последние количества сохранены на экране; повторное подключение автоматически.';
 $('rows').className=kind==='active'||kind==='stopped'?'':'muted';
}
function render(){
 status();
 const query=$('search').value.trim().toLocaleLowerCase('ru');
 const rows=state.rows.filter(row=>(!$('changed').checked||row.updates>0)&&(!query||`${row.name||''} ${row.item_id}`.toLocaleLowerCase('ru').includes(query)));
 const sort=$('sort').value;
 rows.sort((a,b)=>sort==='count'?b.count-a.count||a.item_id-b.item_id:sort==='id'?a.item_id-b.item_id:(a.name||'ЯЯЯ').localeCompare(b.name||'ЯЯЯ','ru')||a.item_id-b.item_id);
 table('rows',rows,row=>[row.name||'Неизвестный материал',row.item_id,number(row.initial_count),number(row.count),signed(row.delta),timestamp(row.updated_at)],state.rows.length?'Совпадений нет':'Ожидание исходного снимка');
 $('summary').textContent=`Показано ${rows.length} из ${state.rows.length} материалов · ${state.history_total} изменений · пропущено/отклонено: ${state.issues}`;
 const historyQuery=$('history-search').value.trim().toLocaleLowerCase('ru');
 const history=state.history.filter(row=>!historyQuery||`${row.name||''} ${row.item_id}`.toLocaleLowerCase('ru').includes(historyQuery));
 table('history-rows',history,row=>[timestamp(row.timestamp),elapsed(row.source_elapsed_ms),row.name||'Неизвестный предмет',row.item_id,row.previous_display||number(row.previous_count),row.current_display||number(row.count),row.action||signed(row.delta)],'Изменений пока нет');
 for(const [id,prefix,title] of [[202,'currency','Мора'],[201,'primogems','Примогемы']]){
  const currency=(state.currencies||[]).find(row=>row.item_id===id);
  $(`${prefix}-summary`).textContent=currency?`${title}: ${number(currency.count)} · изменение от начала: ${signed(currency.count-currency.initial_count)}`:`${title}: ожидание исходного баланса`;
  $(`${prefix}-time`).textContent=`Последнее обновление: ${timestamp(currency?.updated_at)}`;
 }
 const achievementRows=state.achievements||[];
 const categorySelect=$('achievements-category');
 const categories=[...new Set(achievementRows.map(r=>r.category).filter(Boolean))].sort((a,b)=>a.localeCompare(b,'ru'));
 const signature=JSON.stringify(categories);
 if(categorySelect.dataset.signature!==signature){
  const selected=categorySelect.value;
  categorySelect.replaceChildren();
  for(const value of ['',...categories]){const option=document.createElement('option');option.value=value;option.textContent=value||'Все категории';categorySelect.append(option);}
  categorySelect.value=categories.includes(selected)?selected:'';categorySelect.dataset.signature=signature;
 }
 const achievementQuery=$('achievements-search').value.trim().toLocaleLowerCase('ru');
 const achievementStatus=$('achievements-status').value;
 const achievements=achievementRows.filter(r=>(!achievementQuery||`${r.name||''} ${r.achievement_id}`.toLocaleLowerCase('ru').includes(achievementQuery))&&(!categorySelect.value||r.category===categorySelect.value)&&({all:true,unfinished:r.status_code===1,completed:r.status_code>=2,unclaimed:r.status_code===2,claimed:r.status_code===3}[achievementStatus]));
 table('achievements-rows',achievements,r=>[r.name||'Неизвестное достижение',r.achievement_id,r.category||'—',r.progress_display,timestamp(r.updated_at)],'Достижений по фильтрам нет');
 $('achievements-summary').textContent=`Показано ${achievements.length} из ${achievementRows.length} записей · выполнено ${achievementRows.filter(r=>r.status_code>=2).length}.`;
 $('adventure-summary').textContent=state.adventure?`Ранг приключений: ${number(state.adventure.rank)} · опыт: ${number(state.adventure.exp)}`:'Ранг и опыт приключений: ожидание исходного состояния';
 $('adventure-time').textContent=state.adventure?`Последнее обновление: ${timestamp(state.adventure.updated_at)}`:'';
 const characterQuery=$('characters-search').value.trim().toLocaleLowerCase('ru');
 const characters=(state.characters||[]).filter(row=>!characterQuery||`${row.name||''} ${row.avatar_id}`.toLocaleLowerCase('ru').includes(characterQuery));
 table('characters-rows',characters,row=>[row.name||'Неизвестный персонаж',row.avatar_id,row.guid,row.character_level??'—',number(row.character_exp),row.ascension??'—',row.constellation==null?'—':`C${row.constellation}`,(row.talents||[]).map(t=>`${t.name||`Талант ID ${t.skill_id}`}: ${t.level}`).join('\n')||'—',row.level??'—',number(row.exp),timestamp(row.updated_at)],'Ожидание профиля персонажей');
 const itemQuery=$('items-search').value.trim().toLocaleLowerCase('ru');
 const items=(state.items||[]).filter(row=>!itemQuery||`${row.name||''} ${row.item_id} ${row.guid}`.toLocaleLowerCase('ru').includes(itemQuery));
 const kinds={artifact:'Артефакт',weapon:'Оружие',furniture:'Мебель',beyond_material:'Материал Астрального предела',beyond_facility:'Декор Астрального предела',tps_weapon:'TPS-оружие',unknown:'Неизвестно'};
 table('items-rows',items,row=>[row.name||'Неизвестный предмет',row.item_id,kinds[row.kind]||row.kind,row.guid,row.level??'—',row.kind==='weapon'&&row.refinement!=null?`R${row.refinement}`:'—',row.exp??'—',row.count??'—',timestamp(row.updated_at)],'Предметов пока нет');
 $('items-summary').textContent=`Показано ${items.length} из ${(state.items||[]).length} экземпляров. Отдельная строка на каждый GUID.`;
 $('history-summary').textContent=`Показано ${history.length} из последних ${state.history.length} событий · всего ${state.history_total}. Новые события сверху. Timestamp совпадает с журналом и консолью; отдельный столбец — время от начала захвата.`;
 $('history-tab').textContent=`История изменений (${state.history_total})`;
}
function tab(name){for(const section of ['materials','items','characters','achievements','history']){$(`${section}-panel`).hidden=section!==name;$(`${section}-tab`).setAttribute('aria-pressed',String(section===name));}}
for(const name of ['materials','items','characters','achievements','history'])$(`${name}-tab`).addEventListener('click',()=>tab(name));
$('achievements-search').addEventListener('input',render);
$('achievements-category').addEventListener('change',render);
$('achievements-status').addEventListener('change',render);
$('characters-search').addEventListener('input',render);
$('items-search').addEventListener('input',render);
$('search').addEventListener('input',render);$('sort').addEventListener('change',render);$('changed').addEventListener('change',render);$('history-search').addEventListener('input',render);
$('clear').addEventListener('click',()=>{$('search').value='';$('sort').value='name';$('changed').checked=false;render();});
$('history-clear').addEventListener('click',()=>{$('history-search').value='';render();});
async function poll(){
 try{
  const response=await fetch(`/api/state?after=${state.revision}`,{cache:'no-store',signal:AbortSignal.timeout(2500)});
  if(response.status===200){state=await response.json();connected=true;render();}
  else if(response.status===204){const recovered=!connected;connected=true;if(recovered)render();}
  else throw new Error(`HTTP ${response.status}`);
 }catch{connected=false;status();}
 setTimeout(poll,500);
}
poll();
