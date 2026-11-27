// Node-only rune loader; the pixel core is the only replacement.
import { createRequire, registerHooks } from 'node:module';
import { readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
const require = createRequire(new URL('../../ui/package.json', import.meta.url));
const { compileModule } = require('svelte/compiler');
const ts = require('typescript');
export const hooks = registerHooks({
  resolve(specifier, context, next) {
    try { return next(specifier, context); }
    catch (error) { if(specifier.startsWith('.') && (!/\.[a-z]+$/.test(specifier) || specifier.endsWith('.svelte'))) return next(specifier+'.ts',context); throw error; }
  },
  load(url, context, next) {
    if(url.endsWith('/core/core.ts')) return {format:'module',shortCircuit:true,source:'export const newCore=()=>Promise.resolve({});export const isDesktop=()=>false;export const isHosted=()=>false;'};
    let result=next(url,context);
    const baseline=process.env.GRAINATLAS_SYNC_BASELINE_DIR;
    const sourcePath=baseline && path.join(baseline,path.basename(new URL(url).pathname));
    if(sourcePath && url.includes('/ui/src/lib/') && existsSync(sourcePath)) {
      result={...result,format:'module',source:ts.transpileModule(readFileSync(sourcePath,'utf8'),{compilerOptions:{target:ts.ScriptTarget.ESNext,module:ts.ModuleKind.ESNext}}).outputText};
    }
    if(url.endsWith('.svelte.ts') || url.endsWith('/remote.ts')) {
      const source=ts.transpileModule(String(result.source),{compilerOptions:{target:ts.ScriptTarget.ESNext,module:ts.ModuleKind.ESNext}}).outputText;
      return {format:'module',shortCircuit:true,source:url.endsWith('.svelte.ts')?compileModule(source,{filename:new URL(url).pathname,generate:'client'}).js.code:source};
    } return result;
  },
});
