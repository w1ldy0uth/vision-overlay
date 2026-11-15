export function inventoryWindow(count,columns,pitch,offset,viewport,overscan=3) {
  columns=Math.max(1,columns);pitch=Math.max(1,pitch);
  const lines=Math.ceil(count/columns);
  const first=Math.min(Math.max(0,lines-1),Math.max(0,Math.floor(offset/pitch)));
  const start=Math.max(0,first-overscan);
  const end=Math.min(lines,Math.max(start+1,Math.ceil((Math.max(0,offset)+viewport)/pitch)+overscan));
  return {start:start*columns,end:Math.min(count,end*columns),top:start*pitch,bottom:Math.max(0,lines-end)*pitch};
}
export const inventoryKey=row=>`${row.source_collection??'characters'}:${row.entity_key}`;
export function anchorShift(previous,next,index,columns) {
  if(!previous.length||!next.length)return 0;
  for(let i=Math.min(index,previous.length-1);i<previous.length;i++) {
    const at=next.findIndex(r=>inventoryKey(r)===inventoryKey(previous[i]));
    if(at>=0)return Math.floor(at/columns)-Math.floor(i/columns);
  }
  return 0;
}
