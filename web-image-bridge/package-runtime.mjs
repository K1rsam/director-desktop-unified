import fs from 'node:fs';import path from 'node:path';import{fileURLToPath}from'node:url';import{createHash}from'node:crypto';
const root=path.dirname(fileURLToPath(import.meta.url)),repo=path.dirname(root),pin=JSON.parse(fs.readFileSync(path.join(root,'vendor-source.json')));
for(const f of pin.files){const p=path.join(root,'vendor/opencli',f.path);if(createHash('sha256').update(fs.readFileSync(p)).digest('hex')!==f.sha256)throw Error('Pinned source changed: '+f.path);}
const destination=process.argv[2]?path.resolve(process.argv[2]):path.join(repo,'apps/desktop/src-tauri/resources/opencli');
fs.mkdirSync(destination,{recursive:true});if(fs.lstatSync(destination).isSymbolicLink())throw Error('Resource directory must not be a link');
for(const file of ['bridge.mjs','vendor-source.json','package.json','package-lock.json'])fs.copyFileSync(path.join(root,file),path.join(destination,file));
for(const dir of ['vendor','node_modules'])fs.cpSync(path.join(root,dir),path.join(destination,dir),{recursive:true,filter:p=>!p.endsWith('.log')});
const runtimeSha=createHash('sha256');for(const file of ['bridge.mjs','vendor-source.json','package-lock.json'])runtimeSha.update(fs.readFileSync(path.join(root,file)));
const runtimePin=runtimeSha.digest('hex')+'\n';
fs.writeFileSync(path.join(destination,'runtime.sha256'),runtimePin);
fs.writeFileSync(path.join(root,'runtime.sha256'),runtimePin);
const files=[];function walk(dir){for(const entry of fs.readdirSync(dir,{withFileTypes:true})){const p=path.join(dir,entry.name);if(entry.isDirectory())walk(p);else if(entry.name!=='runtime-manifest.json')files.push({path:path.relative(destination,p).replaceAll('\\','/'),sha256:createHash('sha256').update(fs.readFileSync(p)).digest('hex')});}}walk(destination);
fs.writeFileSync(path.join(destination,'runtime-manifest.json'),JSON.stringify({version:'director-opencli-runtime/v1',sourceCommit:pin.commit,files},null,2));console.log(JSON.stringify({packaged:true,files:files.length,destination}));
