import { test, expect, vi } from 'vitest';
test('worker replies route by id; worker failure rejects pending jobs and permits a fallback retry',async()=>{
  let worker: {onmessage:(event:unknown)=>void;onerror:()=>void;terminate:()=>void;sent:any[]} | undefined;
  class FakeWorker {
    onmessage=(_event:unknown)=>{};onerror=()=>{};terminate=vi.fn();sent:any[]=[];
    constructor(){worker=this;}
    postMessage(value:unknown){this.sent.push(value);}
  }
  vi.stubGlobal('Worker',FakeWorker);
  vi.resetModules();
  try {
    const {syncCompute}=await import('./sync-compute');
    const one=syncCompute({op:'parse',text:'1'}),two=syncCompute({op:'parse',text:'2'});
    const [a,b]=worker!.sent;
    worker!.onmessage({data:{id:b.id,value:2}});worker!.onmessage({data:{id:a.id,value:1}});
    expect(await Promise.all([one,two])).toEqual([1,2]);
    const failed=syncCompute({op:'parse',text:'3'});
    const rejection=expect(failed).rejects.toThrow('Changes are kept');worker!.onerror();await rejection;
    expect(worker!.terminate).toHaveBeenCalledOnce();expect(await syncCompute({op:'parse',text:'4'})).toBe(4);
  } finally {vi.unstubAllGlobals();vi.resetModules();}
});
