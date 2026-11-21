#!/usr/bin/env node
// Read-only. Node >=22.18 (native TypeScript stripping); no package dependencies.
import { readFile, readdir, stat } from 'node:fs/promises';
import path from 'node:path';
import { spotDiameter, nextSpotDiameter, areaCalibration } from '../ui/src/lib/spot-size.ts';

const args = process.argv.slice(2), json = args.includes('--json');
const paths = args.filter(a => a !== '--json');
if (!paths.length || paths.some(a => a.startsWith('--'))) {
  process.stderr.write('Usage: node scripts/scan_spot_sizes.mjs [--json] <project.json|file.gaproj|folder.grainatlas|store-folder> ...\n');
  process.exitCode = 1;
} else {
  const documents = [], errors = [];
  const seen = new Set();
  async function read(input) {
    const file = path.resolve(input);
    if (seen.has(file)) return;
    seen.add(file);
    try {
      if ((await stat(file)).isDirectory()) {
        const entries = (await readdir(file,{withFileTypes:true})).sort((a,b)=>a.name.localeCompare(b.name));
        if (entries.some(e=>e.name==='project.json')) return await read(path.join(file,'project.json'));
        for (const e of entries) if (e.isFile() && /\.(json|gaproj)$/.test(e.name)) await read(path.join(file,e.name));
        return;
      }
      let value = JSON.parse(await readFile(file,'utf8')), key = null;
      // Desktop envelope -> audited storage entry -> JSON app value.
      for (let n=0;n<8;n++) {
        if (typeof value === 'string') { value = JSON.parse(value); continue; }
        if (value && typeof value === 'object' && typeof value.contents === 'string') { key=value.key ?? key; value=value.contents; continue; }
        if (value && typeof value === 'object' && typeof value.value === 'string') { value=value.value; continue; }
        break;
      }
      if (value && typeof value === 'object') documents.push({file,key,value});
    } catch (error) { errors.push(`${file}: ${error.message}`); }
  }
  for (const input of paths) await read(input);
  const projects = new Map();
  for (const {value} of documents) { const p=value.project ?? value; if (p.grainatlas_ui) projects.set(p.uuid,p); }
  const reports = [];
  function report(doc, project, spots, snapshot) {
    const known = project.areas ?? [], ids = new Set([...known.map(a=>a.id),...spots.map(s=>s.area_id)]);
    const areas = [...ids].map(id => {
      const area = known.find(a=>a.id===id), here=spots.filter(s=>s.area_id===id), histogram={};
      for(const s of here) { const label=s.diameter_um===undefined?'missing':JSON.stringify(s.diameter_um); histogram[label]=(histogram[label]??0)+1; }
      const record = s => ({id:s.id,name:s.name,stored:s.diameter_um ?? null,...(s.diameter_um===undefined?{missing:true}:{})});
      return {id,name:area?.name??id,spotCount:here.length,histogram,
        invalidSizes:here.filter(s=>spotDiameter(s)===null).map(record),
        drawnSizeDifferences:here.filter(s=>!Object.is(spotDiameter(s),s.diameter_um)).map(s=>({...record(s),drawn:spotDiameter(s)})),
        calibrationUmPerPx:areaCalibration(area??null,area?.mosaic??null),
        nextSpotSize:nextSpotDiameter(spots,id,project.settings?.default_diameter_um??25)};
    });
    reports.push({source:doc.file,key:doc.key,project:project.name??project.uuid??'Unnamed project',uuid:project.uuid??null,snapshot,areas});
  }
  for (const doc of documents) {
    const value=doc.value, p=value.project??value;
    if (p.grainatlas_ui) report(doc,p,p.grainatlas_ui.spots??[],'project');
    for (const snapshot of ['base','base_seen','pending']) if(Array.isArray(value[snapshot]?.spots)) {
      const context = projects.get(value.uuid) ?? {uuid:value.uuid,name:value.name,areas:[],settings:{}};
      report(doc,context,value[snapshot].spots,snapshot);
    }
  }
  if (json) process.stdout.write(JSON.stringify(reports,null,2)+'\n');
  else for (const r of reports) {
    process.stdout.write(`${r.project} [${r.snapshot}] — ${r.source}${r.key?` (${r.key})`:''}\n`);
    for (const a of r.areas) {
      process.stdout.write(`  ${a.name}: ${a.spotCount} spots; sizes ${JSON.stringify(a.histogram)}; next spot ${a.nextSpotSize} µm; calibration ${a.calibrationUmPerPx===null?'not set':`${a.calibrationUmPerPx} µm/px`}\n`);
      for(const s of a.invalidSizes) process.stdout.write(`    no size: ${s.name??s.id} (${s.id}): ${s.missing?'missing':JSON.stringify(s.stored)}\n`);
      for(const s of a.drawnSizeDifferences) process.stdout.write(`    drawn differs: ${s.name??s.id} (${s.id}): stored ${s.missing?'missing':JSON.stringify(s.stored)}, drawn ${JSON.stringify(s.drawn)}\n`);
    }
  }
  if(errors.length) { process.stderr.write(errors.join('\n')+'\n'); process.exitCode=1; }
}
