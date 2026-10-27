// Read current, declared asset dependencies without invalidating unrelated episodes.
import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {readRegistry,validateRegistry} from './assets.mjs';
const [registryPath,bundlePath]=process.argv.slice(2);
try {
 const bundle=JSON.parse(await readFile(bundlePath,'utf8'));
 const {registry,file,sha256}=await readRegistry(registryPath),{ids}=validateRegistry(registry);
 const episodes=new Set(bundle.episodes.map(e=>e.number)),wanted=new Set();
 const projection=new Map(bundle.assets.map(a=>[a.key,a.assetId]));
 function add(id){
  if(wanted.has(id))return;
  const a=ids.get(id);if(!a)throw new Error('Missing declared asset: '+id);
  wanted.add(id);for(const d of a.design.dependencies??[])add(d.assetId);
 }
 for(const e of bundle.episodes)for(const u of e.draft.units)for(const key of u.bindings)add(projection.get(key));
 const relevant={id:registry.id,sourceHash:registry.sourceHash,importProfile:registry.importProfile??null,
  productionScope:registry.productionScope??null,
  assets:[...wanted].sort().map(id=>ids.get(id)),
  requests:registry.requests.filter(r=>r.uses.some(u=>episodes.has(u.episode)))};
 const stable=JSON.stringify(relevant,(_,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.keys(v).sort().map(k=>[k,v[k]])):v);
 console.log(JSON.stringify({fingerprint:createHash('sha256').update(stable).digest('hex'),snapshot:{path:file,sha256}}));
} catch(error){console.error(error.code??'DEPENDENCIES',error.message);process.exitCode=1;}
