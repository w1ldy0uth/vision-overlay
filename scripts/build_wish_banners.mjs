// Read pinned source as syntax/data only. Never execute downloaded JavaScript.
import fs from 'node:fs';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
import { parsers } from '../web/node_modules/prettier/plugins/acorn.mjs';
const root=process.argv[2];
if(!root)throw Error('Pass the pinned paimon-moe source directory');
const revision=execFileSync('git',['-C',root,'rev-parse','HEAD'],{encoding:'utf8'}).trim();
if(revision!=='4b873a6decad0384665084c69e9927a9fa266555')throw Error('Source revision differs from reviewed banner data');
function ast(file, variable) {
  const source=parsers.acorn.parse(fs.readFileSync(file,'utf8'));
  for(const outer of source.body) {
    const statement=outer.type==='ExportNamedDeclaration'?outer.declaration:outer;
    if(statement?.type==='VariableDeclaration')for(const d of statement.declarations)if(d.id.name===variable)return d.init;
  }
  throw Error('Missing data declaration');
}
function literal(n) {
  if(n.type==='Literal')return n.value;
  if(n.type==='ArrayExpression')return n.elements.map(literal);
  if(n.type==='ObjectExpression')return Object.fromEntries(n.properties.map(p=>{
    if(p.type!=='Property'||p.computed||p.method)throw Error('Unsupported data property');
    return [p.key.name??p.key.value,literal(p.value)];
  }));
  throw Error('Nonliteral banner data');
}
const source=`${root}/src/data/banners.js`, data=literal(ast(source,'banners'));
const translations=JSON.parse(fs.readFileSync(`${root}/src/locales/items/ru.json`,'utf8'));
const labels={};
// Only names/IDs from item definitions; never interpret the remaining expressions.
for(const [file,variable] of [['characters','characters'],['weaponList','weaponList']]) {
  const p=`${root}/src/data/${file}.js`, node=ast(p,variable);
  for(const entry of node.properties) {
    const fields=Object.fromEntries(entry.value.properties.filter(p=>p.type==='Property').filter(p=>['id','name'].includes(p.key.name)).map(p=>[p.key.name,literal(p.value)]));
    if(fields.id&&fields.name)labels[fields.id]=translations[fields.name]??null;
  }
}
const rows=[];
for(const [key,group] of [['characters','characters'],['weapons','weapons'],['chronicled','chronicled']]) {
  for(const b of data[key])rows.push({group,start:b.start,end:b.end,startGlobal:b.timezoneDependent===true,endGlobal:b.timezoneDependentEnd===true,featured:b.featured.map(id=>({key:id,name:labels[id]??null}))});
}
const output={schemaVersion:1,source:'https://github.com/MadeBaruna/paimon-moe',revision:'4b873a6decad0384665084c69e9927a9fa266555',sha256:crypto.createHash('sha256').update(fs.readFileSync(source)).digest('hex'),rows};
fs.writeFileSync('web/src/data/wish-banners.json',JSON.stringify(output));
console.log(`${rows.length} banner periods; ${rows.flatMap(r=>r.featured).filter(x=>!x.name).length} unresolved translated labels (left unknown)`);
