// Version-bound, reversible deployment; never touches bridge state or credentials.
const fs=require('fs'),path=require('path'),crypto=require('crypto');
const source=path.resolve(__dirname,'../overrides');
const target=path.resolve(process.argv[2]||path.join(process.env.LOCALAPPDATA,'Programs/Director-3008-ChatGPT-Bridge/671815e'));
const hash=b=>crypto.createHash('sha256').update(b).digest('hex');
const manifest=JSON.parse(fs.readFileSync(path.join(source,'manifest.json'),'utf8'));
const pkg=JSON.parse(fs.readFileSync(path.join(target,'package.json'),'utf8'));
if(pkg.name!=='chatgpt-browser-bridge-node'||pkg.version!==manifest.componentVersion)throw Error('Unsupported Chrome bridge version');
const pending=manifest.files.map(f=>{
 const relative=path.normalize(f.path);if(path.isAbsolute(relative)||relative.startsWith('..'))throw Error('Unsafe overlay path');
 const destination=path.join(target,relative);if(fs.lstatSync(destination).isSymbolicLink())throw Error('Linked overlay target');
 const before=fs.readFileSync(destination),after=fs.readFileSync(path.join(source,relative));
 const prior=f.previousAfterSha256s||[];
 if(!Array.isArray(prior)||prior.some(s=>typeof s!=='string'||!/^[a-f0-9]{64}$/.test(s)))throw Error('Invalid pinned overlay history');
 if(hash(after)!==f.afterSha256||![f.beforeSha256,f.previousAfterSha256,...prior,f.afterSha256].includes(hash(before)))throw Error('Chrome component changed; refusing overwrite: '+relative);
 return {f,destination,before,after,unchanged:hash(before)===f.afterSha256};
});
for(const item of pending){
 if(item.unchanged)continue;
 const backup=path.join(target,'.director-native-recovery-backup',item.f.path);
 fs.mkdirSync(path.dirname(backup),{recursive:true});
 if(!fs.existsSync(backup))fs.writeFileSync(backup,item.before,{flag:'wx'});
 if(hash(fs.readFileSync(backup))!==item.f.beforeSha256)throw Error('Original backup changed');
 const temporary=item.destination+'.director-recovery.tmp';fs.writeFileSync(temporary,item.after,{flag:'wx'});fs.renameSync(temporary,item.destination);
}
console.log(JSON.stringify({version:manifest.version,files:pending.length,changed:pending.filter(p=>!p.unchanged).length,stateUntouched:true}));
