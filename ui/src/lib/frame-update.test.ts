import { test, expect, vi } from 'vitest';
import { FrameUpdate } from './frame-update';
test('latest progress/status per frame, explicit flush and cancellation',()=>{
  let frame: (()=>void)|undefined;
  vi.stubGlobal('requestAnimationFrame',(run:()=>void)=>{frame=run;return 1;});
  vi.stubGlobal('cancelAnimationFrame',()=>{frame=undefined;});
  try {
    const queue=new FrameUpdate(),seen:number[]=[];
    for(let i=0;i<100;i++)queue.enqueue(()=>seen.push(i));
    expect(seen).toEqual([]);frame!();expect(seen).toEqual([99]);
    queue.enqueue(()=>seen.push(100));queue.flush();expect(seen).toEqual([99,100]);expect(frame).toBeUndefined();
    queue.enqueue(()=>seen.push(101));queue.cancel();expect(frame).toBeUndefined();expect(seen).toEqual([99,100]);
  } finally { vi.unstubAllGlobals(); }
});
