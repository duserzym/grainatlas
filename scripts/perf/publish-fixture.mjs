import { createHash } from 'node:crypto';
/** Minimal valid STORE zip; generated outside measured phases, invented bytes only. */
export function publishFixture(project) {
  const files=[{path:'project.json',bytes:Buffer.from(JSON.stringify(project))},...Array.from({length:32},(_,i)=>({path:`synthetic/${i}.bin`,bytes:Buffer.alloc(4096,i)}))];
  const atlas={project_id:project.uuid,files:files.map(f=>({path:f.path,size:f.bytes.length,sha256:createHash('sha256').update(f.bytes).digest('hex')}))};
  const entries=[{path:'atlas.json',bytes:Buffer.from(JSON.stringify(atlas))},...files];
  const local=[],central=[];let offset=0;
  for(const file of entries) {
    const name=Buffer.from(file.path);let crc=0xffffffff;
    for(const byte of file.bytes){crc^=byte;for(let i=0;i<8;i++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}
    crc=(crc^0xffffffff)>>>0;
    const h=Buffer.alloc(30+name.length);h.writeUInt32LE(0x04034b50);h.writeUInt16LE(20,4);h.writeUInt32LE(crc,14);h.writeUInt32LE(file.bytes.length,18);h.writeUInt32LE(file.bytes.length,22);h.writeUInt16LE(name.length,26);name.copy(h,30);
    const c=Buffer.alloc(46+name.length);c.writeUInt32LE(0x02014b50);c.writeUInt16LE(20,4);c.writeUInt16LE(20,6);c.writeUInt32LE(crc,16);c.writeUInt32LE(file.bytes.length,20);c.writeUInt32LE(file.bytes.length,24);c.writeUInt16LE(name.length,28);c.writeUInt32LE(offset,42);name.copy(c,46);
    local.push(h,file.bytes);central.push(c);offset+=h.length+file.bytes.length;
  }
  const directory=Buffer.concat(central),end=Buffer.alloc(22);end.writeUInt32LE(0x06054b50);end.writeUInt16LE(entries.length,8);end.writeUInt16LE(entries.length,10);end.writeUInt32LE(directory.length,12);end.writeUInt32LE(offset,16);
  return {atlas,zip:new Uint8Array(Buffer.concat([...local,directory,end]))};
}
