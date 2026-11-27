import { test, expect, vi } from 'vitest';
import { HostedDraftReports } from './hosted-draft-reports';
import type { Draft } from './drafts';
test('hosted draft changes reuse saved records in order, coalesce per frame, and reload after deletion',()=>{
  let frame: (()=>void)|undefined;
  vi.stubGlobal('requestAnimationFrame',(run:()=>void)=>{frame=run;return 1;});
  vi.stubGlobal('cancelAnimationFrame',()=>{frame=undefined;});
  try {
    const a={uuid:'a'} as Draft,b={uuid:'b'} as Draft,load=vi.fn(()=>[a,b]),send=vi.fn();
    const reports=new HostedDraftReports(load,send);
    reports.changed();frame!();expect(load).toHaveBeenCalledTimes(1);
    const changed={...a,name:'latest'};
    reports.changed({...a,name:'old'});reports.changed(changed);
    expect(send).toHaveBeenCalledTimes(1);frame!();expect(load).toHaveBeenCalledTimes(1);expect(send).toHaveBeenLastCalledWith([changed,b]);
    reports.changed();frame!();expect(load).toHaveBeenCalledTimes(2);
    reports.changed(changed);reports.dispose();expect(frame).toBeUndefined();
  } finally { vi.unstubAllGlobals(); }
});
