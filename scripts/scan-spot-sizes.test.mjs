import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, mkdir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';

const scan = new URL('./scan_spot_sizes.mjs', import.meta.url);
test('read-only scanner unwraps synthetic projects, copies and draft snapshots and never writes', async () => {
  const root = await mkdtemp(path.join(tmpdir(),'synthetic-sizes-'));
  try {
    const spots = [{id:'one',name:'Synthetic one',area_id:'a',diameter_um:20,created:'2026-01-01'}, {id:'two',name:'Synthetic two',area_id:'a',diameter_um:null}, {id:'three',name:'Synthetic three',area_id:'b',diameter_um:'25'}];
    const project = {uuid:'synthetic',name:'Synthetic mount',settings:{default_diameter_um:35},areas:[{id:'a',name:'Synthetic A',pixel_size_nm:200},{id:'b',name:'Synthetic B',pixel_size_nm:0}],grainatlas_ui:{spots}};
    const file = path.join(root,'synthetic.gaproj'), folder = path.join(root,'synthetic.grainatlas'), store=path.join(root,'store');
    await mkdir(folder); await mkdir(store);
    const files = [file,path.join(folder,'project.json'),path.join(store,'entry.json'),path.join(store,'draft.json')];
    const contents = [JSON.stringify(project),JSON.stringify(project),JSON.stringify({key:'grainatlas.copy.synthetic',contents:JSON.stringify({value:JSON.stringify({project})})}),JSON.stringify({key:'grainatlas.draft.synthetic',contents:JSON.stringify({value:JSON.stringify({uuid:'synthetic',name:'Synthetic mount',base:{spots},base_seen:{spots},pending:{spots}})})})];
    for (let i=0;i<files.length;i++) await writeFile(files[i],contents[i]);
    const reports = JSON.parse(execFileSync(process.execPath,[scan.pathname,'--json',file,folder,store],{encoding:'utf8'}));
    assert.equal(reports.length,6);
    const p = reports[0]; assert.equal(p.project,'Synthetic mount'); assert.equal(p.areas[0].spotCount,2);
    assert.equal(p.areas[0].nextSpotSize,20); assert.equal(p.areas[1].nextSpotSize,20);
    assert.equal(p.areas[0].calibrationUmPerPx,.2); assert.equal(p.areas[1].calibrationUmPerPx,null);
    assert.equal(p.areas[0].invalidSizes[0].id,'two'); assert.equal(p.areas[1].drawnSizeDifferences[0].stored,'25');
    assert.equal(p.areas[0].drawnSizeDifferences.length,0); // null is explicitly unset, no guessed diameter
    assert.equal(reports.filter(r=>r.snapshot==='pending').length,1);
    assert.match(execFileSync(process.execPath,[scan.pathname,file],{encoding:'utf8'}),/Synthetic A: 2 spots/);
    for(let i=0;i<files.length;i++) assert.equal(await readFile(files[i],'utf8'),contents[i]);
  } finally { await rm(root,{recursive:true,force:true}); }
});

test('scanner chooses the area mode even when a minority size has a creation time', async () => {
  const root = await mkdtemp(path.join(tmpdir(),'synthetic-size-mode-'));
  try {
    const spots = Array.from({length:59},(_,i)=>({id:`synthetic-${i}`,area_id:'a',diameter_um:40,...(i%2?{created:''}:{})}));
    spots.push({id:'synthetic-local',area_id:'a',diameter_um:25,created:'2026-11-10'});
    const file = path.join(root,'synthetic.gaproj');
    const contents = JSON.stringify({name:'Synthetic mode',areas:[{id:'a'},{id:'empty-area'}],settings:{default_diameter_um:35},grainatlas_ui:{spots}});
    await writeFile(file,contents);
    const [report] = JSON.parse(execFileSync(process.execPath,[scan.pathname,'--json',file],{encoding:'utf8'}));
    assert.deepEqual(report.areas.map(a=>a.nextSpotSize),[40,40]);
    assert.equal(await readFile(file,'utf8'),contents);
  } finally { await rm(root,{recursive:true,force:true}); }
});
