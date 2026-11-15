import avatarRarities from './data/wish-avatar-rarities.json' with {type:'json'};

// Local capture identities remain separate from IDs owned by the history API.
export function wishRecords(data, catalog) {
  const zone=data.last_import?.metadata?.region_time_zone;
  const local=(data.live_enabled?data.live_records??[]:[]).map(r=>{
    const quality=catalog.avatarMetadata?.[r.item_id]?.qualityType;
    const stars=quality==='QUALITY_PURPLE'?4:['QUALITY_ORANGE','QUALITY_ORANGE_SP'].includes(quality)?5:
      catalog.rarities?.[r.item_id]??avatarRarities.rows[r.item_id];
    const ms=Number(r.observed_ms);
    return {...r,id:`live:${r.live_id}`,live:true,rank_type:stars?String(stars):'',
      name:catalog.avatarNames?.[r.item_id]??catalog.names?.[r.item_id]??`Предмет ${r.item_id}`,
      time:Number.isInteger(zone)?new Date(ms+zone*3600000).toISOString().slice(0,19).replace('T',' '):'',
      display_time:new Date(ms).toLocaleString('ru-RU'),lang:'ru'};
  });
  return [...local,...data.records];
}

const numeric=(a,b)=>a.length-b.length||a.localeCompare(b);
export function compareWishes(a,b) {
  if(!a.live&&!b.live)return numeric(a.id,b.id);
  if(a.live&&b.live)return Number(a.observed_ms)-Number(b.observed_ms)||Number(a.source_sequence)-Number(b.source_sequence)||a.result_index-b.result_index;
  // The store has already consumed matching API prefixes; remaining captured
  // rows are later than every imported row in their pool.
  return a.live?1:-1;
}
