// Drives the COMMITTED fixture through a live kernel, to verify the claims
// the fixture's own prose makes before they are preregistered.
import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os'; import { join } from 'node:path';
const ROOT='/Users/mark/Sites/brain-server';
const PORT=8985, base=`http://127.0.0.1:${PORT}`;
// Parse the corpus table out of the committed fixture (no second copy).
// SCOPED to the `## The corpus` section on purpose: the first draft scanned the
// whole file and matched 19 rows, because the explanation tables are numbered
// the same way. A parser that reads a doc's PROSE as its data is a parser that
// will one day seed a corpus with a sentence about corpora.
const md=readFileSync(join(ROOT,'shell/eval/abstention_queries.md'),'utf8');
const section=md.slice(md.indexOf('## The corpus'));
const corpusSection=section.slice(0, section.indexOf('\n## ',1));
const CORPUS:string[]=[];
for(const line of corpusSection.split('\n')){
  const m=/^\|\s*(\d+)\s*\|\s*(.+?)\s*\|\s*$/.exec(line);
  if(m) CORPUS.push(m[2]!);
}
console.log(`corpus rows parsed from the fixture: ${CORPUS.length}`);
async function health(c:any){for(let i=0;i<480;i++){if(c.exitCode!==null)throw new Error('early exit');
 try{if((await fetch(`${base}/health`)).ok)return;}catch{}
 await new Promise(r=>setTimeout(r,250));}throw new Error('unhealthy');}
const dir=mkdtempSync(join(tmpdir(),'abstfix-'));
const env={...process.env};for(const k of ['AUTH_TOKEN','AUTH_TOKEN_FILE','BRAIN_TOKEN','BRAIN_TOKEN_FILE'])delete env[k];
const child=spawn(join(ROOT,'target/debug/brain-server'),[],{cwd:ROOT,env:{...env,
 BIND_HOST:'127.0.0.1',BIND_PORT:String(PORT),BRAIN_DB_PATH:join(dir,'brain.db'),
 BRAIN_WRITE_POSTURE:'open',BRAIN_AUDIT_READ_EVENTS:'off'},stdio:['ignore','ignore','inherit']});
try{
 await health(child);
 for(const [i,t] of CORPUS.entries()){const r=await fetch(`${base}/add`,{method:'POST',
  headers:{'content-type':'application/json'},body:JSON.stringify({text:t,title:`d${i}`})});
  if(!r.ok)throw new Error(`seed ${i}: ${r.status}`);}
 const qs=[['A1','entries'],['A1','contract entries'],['C1','amendments'],
           ['C2','retention schedule'],['C3','audit cycle']];
 // BOTH arms, because a control that returns nothing under the filter but
 // also nothing WITHOUT it is vacuous: it would "pass" for the wrong reason.
 console.log('\nid  query'.padEnd(30),'arm'.padEnd(22),'decision'.padEnd(16),'ov'.padEnd(6),'gap'.padEnd(9),'lex'.padEnd(6),'score'.padEnd(9),'hits');
 for(const [id,q] of qs){
  for(const [label,extra] of [['filtered',{min_relevance:'high'}],['unfiltered',{}]] as const){
   const r=await fetch(`${base}/recall`,{method:'POST',headers:{'content-type':'application/json'},
    body:JSON.stringify({v:1,query:q,limit:5,provenance:true,graph:false,...extra})});
   const j:any=await r.json();
   const p=j.hits?.[0]?.provenance?.quality_assessment;
   console.log(`${id}  ${JSON.stringify(q)}`.padEnd(30),label.padEnd(22),String(j.decision).padEnd(16),
    (p?.confidence?.overlap??-1).toFixed(2).padEnd(6),(p?.confidence?.gap??-1).toFixed(4).padEnd(9),
    (p?.confidence?.lexical_density??-1).toFixed(2).padEnd(6),(p?.confidence?.score??-1).toFixed(4).padEnd(9),
    String((j.hits??[]).length)+(j.decision==='low_confidence'?'  <== ABSTAIN':''));
  }
 }
}finally{child.kill('SIGTERM');rmSync(dir,{recursive:true,force:true});}
