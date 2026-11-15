import {useLayoutEffect,useRef,useState} from 'react';
import type {Row} from './api';
import {anchorShift,inventoryWindow} from './virtual-inventory.mjs';

export function useInventoryWindow(rows:Row[],resetKey:string) {
  const ref=useRef<HTMLDivElement>(null);
  const previous=useRef({rows,resetKey,columns:1,pitch:84,index:0});
  const [range,setRange]=useState({start:0,end:60,top:0,bottom:0});
  useLayoutEffect(()=>{
    const root=ref.current;if(!root)return;
    let frame=0;
    const measure=()=>{
      const grid=root.querySelector<HTMLElement>('.inventory-grid,.character-grid');
      const table=root.querySelector('table');
      const sample=grid?.firstElementChild??table?.querySelector('tbody tr');
      if(!sample)return;
      const style=grid?getComputedStyle(grid):null;
      const columns=style?style.gridTemplateColumns.split(' ').filter(Boolean).length:1;
      const gap=style?parseFloat(style.rowGap)||0:0;
      const pitch=sample.getBoundingClientRect().height+gap;
      if(!pitch)return;
      const head=grid?parseFloat(style!.paddingTop)||0:table?.querySelector('thead')?.getBoundingClientRect().height??0;
      const offset=-root.getBoundingClientRect().top-head;
      const next=inventoryWindow(rows.length,columns,pitch,offset,window.innerHeight);
      previous.current={rows,resetKey,columns,pitch,index:Math.max(0,Math.floor(offset/pitch))*columns};
      setRange(old=>Object.keys(next).every(k=>old[k as keyof typeof old]===next[k as keyof typeof next])?old:next);
    };
    const old=previous.current;
    if(old.resetKey!==resetKey) {
      setRange({start:0,end:60,top:0,bottom:0});
      if(root.getBoundingClientRect().top<0)window.scrollBy({top:root.getBoundingClientRect().top-16,behavior:'instant'});
    } else if(old.rows!==rows&&root.getBoundingClientRect().top<0) {
      const shift=anchorShift(old.rows,rows,old.index,old.columns);
      if(shift)window.scrollBy({top:shift*old.pitch,behavior:'instant'});
    }
    const update=()=>{cancelAnimationFrame(frame);frame=requestAnimationFrame(measure);};
    const resize=new ResizeObserver(update);resize.observe(root);
    window.addEventListener('scroll',update,{passive:true});window.addEventListener('resize',update);
    update();
    return()=>{cancelAnimationFrame(frame);resize.disconnect();window.removeEventListener('scroll',update);window.removeEventListener('resize',update);};
  },[rows,resetKey]);
  const visible=range.start>=rows.length?{start:0,end:Math.min(60,rows.length),top:0,bottom:0}:range;
  return {ref,...visible,shown:rows.slice(visible.start,Math.max(visible.start,visible.end))};
}
