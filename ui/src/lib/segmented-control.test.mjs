import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { registerHooks } from 'node:module';
import { compile, compileModule } from 'svelte/compiler';
import { render } from 'svelte/server';
import ts from 'typescript';

const url=new URL('./ui/SegmentedControl.svelte',import.meta.url);
const source=readFileSync(url,'utf8');
const controls=JSON.stringify(new URL('./ui-controls.ts',import.meta.url).href);
const hooks=registerHooks({load(path,context,next){
  if(path===url.href)return {format:'module',shortCircuit:true,
    source:compile(source,{filename:url.pathname,generate:'server'}).js.code.replace("'../ui-controls'",controls)};
  return next(path,context);
}});
const {default:SegmentedControl}=await import(url.href);
test.after(()=>hooks.deregister());

// Run the component's actual handlers without a DOM or a listening browser.
const script=source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1]
  .replace(/^  import .*;\n/gm,'').replace('$props()','props').replace('$bindable()','undefined');
const moduleSource=`import { segmentedIndex } from ${controls};
export function create(props) { ${script}
  return {select,keyboard,get value(){return value;},get active(){return active;},buttons};
}`;
const js=ts.transpileModule(moduleSource,{compilerOptions:{target:ts.ScriptTarget.ESNext,module:ts.ModuleKind.ESNext}}).outputText;
const compiled=compileModule(js,{filename:'segmented-handlers.svelte.ts',generate:'client'}).js.code
  .replaceAll("'svelte/internal/client'",JSON.stringify(import.meta.resolve('svelte/internal/client')));
const {create}=await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
const options=[{value:'click',label:'Click'},{value:'draw',label:'Draw'},{value:'box',label:'Box'}];

test('a rejected controlled selection keeps the parent mode; bound selections still update',()=>{
  const requests=[],h=create({options,value:'draw',label:'Ways to add',onchange:value=>requests.push(value)});
  h.select(0);
  assert.deepEqual(requests,['click']);assert.equal(h.value,'draw');
  const bound=create({options,value:'draw',label:'Ways to add'});bound.select(2);assert.equal(bound.value,'box');
});

test('disabled segments cannot select and radio keyboard navigation skips them',()=>{
  const blocked=options.map(option=>({...option,disabled:option.value!=='draw',title:'Needs image pixels. Use Draw.'}));
  const requests=[],h=create({options:blocked,value:'draw',label:'Ways to add',onchange:value=>requests.push(value)});
  h.select(0);h.select(2);assert.deepEqual(requests,[]);assert.equal(h.active,1);
  let focused=0;h.buttons[1]={focus(){focused++;}};
  for(const key of ['ArrowLeft','ArrowRight','Home','End']) {
    let prevented=0,stopped=0;
    h.keyboard({key,preventDefault(){prevented++;},stopPropagation(){stopped++;}},1);
    assert.equal(prevented,1);assert.equal(stopped,1);assert.equal(h.value,'draw');
  }
  assert.equal(focused,4);assert.deepEqual(requests,['draw','draw','draw','draw']);
  const locked=create({options,value:'draw',label:'Ways to add',disabled:true,onchange:()=>assert.fail('disabled group selected')});
  locked.select(0);locked.keyboard({key:'ArrowRight'},1);
  const empty=create({options:blocked.map(o=>({...o,disabled:true})),value:'draw',label:'Ways to add'});
  empty.keyboard({key:'ArrowRight'},1);assert.equal(empty.active,undefined);
});

test('rendered segments expose disabled reasons, the active Draw radio and one keyboard stop',()=>{
  for(const pixels of [false,true]) {
    const body=render(SegmentedControl,{props:{label:'Ways to add',value:'draw',options:options.map(option=>({
      ...option,disabled:!pixels&&option.value!=='draw',title:!pixels&&option.value!=='draw'?'Needs image pixels. Use Draw.':option.label,
    }))}}).body;
    const buttons=[...body.matchAll(/<button\b[^>]*>/g)].map(match=>match[0]);
    assert.equal(buttons.length,3);
    for(const index of [0,2]) {
      assert.equal(/\sdisabled(?:\s|>|=)/.test(buttons[index]),!pixels);
      if(!pixels)assert.match(buttons[index],/title="Needs image pixels\. Use Draw\."/);
    }
    assert.match(buttons[1],/aria-checked="true"/);assert.match(buttons[1],/tabindex="0"/);
    assert.equal(buttons.filter(button=>/tabindex="0"/.test(button)).length,1);
  }
});
