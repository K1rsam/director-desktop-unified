const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),os=require('node:os'),path=require('node:path'),{spawnSync}=require('node:child_process');
const root=path.resolve(__dirname,'..'),manifest=JSON.parse(fs.readFileSync(path.join(root,'overrides/manifest.json')));
function fixture(){const dir=fs.mkdtempSync(path.join(os.tmpdir(),'director-overlay-'));fs.writeFileSync(path.join(dir,'package.json'),JSON.stringify({name:'chatgpt-browser-bridge-node',version:manifest.componentVersion}));for(const f of manifest.files){fs.mkdirSync(path.dirname(path.join(dir,f.path)),{recursive:true});fs.copyFileSync(path.join(root,'overrides',f.path),path.join(dir,f.path));}return dir;}
const apply=dir=>spawnSync(process.execPath,[path.join(root,'scripts/apply-overrides.cjs'),dir],{encoding:'utf8'});
test('pinned extension and server overlays deploy idempotently without touching runtime state',()=>{
  const dir=fixture();try{fs.writeFileSync(path.join(dir,'state.json'),'private synthetic runtime state');const r=apply(dir);assert.equal(r.status,0,r.stderr);assert.equal(JSON.parse(r.stdout).changed,0);assert.equal(fs.readFileSync(path.join(dir,'state.json'),'utf8'),'private synthetic runtime state');assert.equal(fs.existsSync(path.join(dir,'.director-native-recovery-backup')),false);}finally{fs.rmSync(dir,{recursive:true,force:true});}
});
test('unknown component changes reject the entire batch before any file is written',()=>{
  const dir=fixture();try{const f=manifest.files.at(-1),changed=path.join(dir,f.path);fs.appendFileSync(changed,'\n// unknown modification');const before=manifest.files.map(f=>fs.readFileSync(path.join(dir,f.path)));const r=apply(dir);assert.notEqual(r.status,0);assert.match(r.stderr,/refusing overwrite/);for(let i=0;i<before.length;i++)assert.deepEqual(fs.readFileSync(path.join(dir,manifest.files[i].path)),before[i]);assert.equal(fs.existsSync(path.join(dir,'.director-native-recovery-backup')),false);}finally{fs.rmSync(dir,{recursive:true,force:true});}
});
test('a different component version is never treated as compatible',()=>{
  const dir=fixture();try{fs.writeFileSync(path.join(dir,'package.json'),JSON.stringify({name:'chatgpt-browser-bridge-node',version:'unknown'}));const r=apply(dir);assert.notEqual(r.status,0);assert.match(r.stderr,/Unsupported Chrome bridge version/);}finally{fs.rmSync(dir,{recursive:true,force:true});}
});
