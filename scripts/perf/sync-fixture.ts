import type { Grain, Project, Spot } from '../../ui/src/lib/core/types.ts';
/** Invented geometry only, shared by CPU, integration and browser tests. */
export function syncFixture(template?: Project): Project {
  const original = (template?.grainatlas_ui as { grains?: Record<string, Grain[]> } | undefined)?.grains?.a;
  const left = original?.length ? Math.min(...original.map(g => g.bbox_um[0])) : 0;
  const top = original?.length ? Math.min(...original.map(g => g.bbox_um[1])) : 0;
  const width = original?.length ? Math.max(...original.map(g => g.bbox_um[2])) - left : 600;
  const height = original?.length ? Math.max(...original.map(g => g.bbox_um[3])) - top : 400;
  const rx = width / 50, ry = height / 40;
  const grains = Array.from({ length: 150 }, (_, i): Grain => {
    const x = left + (i % 15 + .5) * width / 15, y = top + (Math.floor(i / 15) + .5) * height / 10;
    return { id: `70000000-0000-4000-8000-${String(i + 1).padStart(12, '0')}`, number: i + 1,
      label: `a/${i + 1}`, sample: 'synthetic', centroid_um: [x, y], bbox_um: [x-rx,y-ry,x+rx,y+ry],
      polygon_um: Array.from({length: 256}, (_, j) => [x + rx * Math.cos(j*Math.PI/128), y + ry*Math.sin(j*Math.PI/128)]),
      area_um2: 377 * width / 600 * height / 400, major_um: 2*rx, minor_um: 2*ry, eq_diameter_um: 22 * Math.sqrt(width / 600 * height / 400), orientation_deg: i % 180,
      mean_intensity: 100, scale_bar_um: 10, manual: false };
  });
  const spots = Array.from({length:500}, (_,i): Spot => ({id:`80000000-0000-4000-8000-${String(i+1).padStart(12,'0')}`,
    area_id:'a',name:`registered-${i}`,sample:'synthetic',grain_id:grains[i%150].id,grain_number:grains[i%150].number,n:i+1,
    x_um:grains[i%150].centroid_um[0],y_um:grains[i%150].centroid_um[1],x2_um:null,y2_um:null,diameter_um:8,kind:'spot',
    comment:'keep',created:'',created_by:'',acquisition_id:`90000000-0000-4000-8000-${String(i+1).padStart(12,'0')}`}));
  const p = template ? structuredClone(template) : {schema_version:1,uuid:'10000000-0000-4000-8000-000000000001',name:'Synthetic',
    package:{name:'Synthetic',path:''},upbdb:{mount_id:'20000000-0000-4000-8000-000000000001'},settings:{},
    areas:[{id:'a',name:'A',sample:'synthetic',params:{tone:{},segment:{},stitch:{},photometric:{},blend:{}}}]} as unknown as Project;
  p.grainatlas_ui = {...(p.grainatlas_ui as object),grains:{a:grains},spots,labels:[],tone:{}};
  return p;
}
