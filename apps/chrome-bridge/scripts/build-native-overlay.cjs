// Keep the separately tested adapter in an existing, byte-pinned ESM module.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const root=path.resolve(__dirname,'..'),overrides=path.join(root,'overrides');
const marker='// BEGIN DIRECTOR NATIVE TRANSPORT ADAPTER (generated)';
const modulePath=path.join(overrides,'extension/background/authPreflight.js');
const read=p=>fs.readFileSync(p,'utf8').replace(/\r\n/g,'\n');
const source=read(path.join(root,'native/extension/background/nativeTransport.js'));
if(/^import\s/m.test(source))throw Error('Native adapter must be self-contained');
const current=read(modulePath),first=current.indexOf(marker);
const base=(first<0?current:current.slice(0,first)).trimEnd();
fs.writeFileSync(modulePath,(base+'\n\n'+marker+'\n'+source.trimEnd()+'\n// END DIRECTOR NATIVE TRANSPORT ADAPTER\n').replace(/\n/g,'\r\n'));
const manifestPath=path.join(overrides,'manifest.json'),manifest=JSON.parse(read(manifestPath));
for(const file of manifest.files){
 const sha=crypto.createHash('sha256').update(fs.readFileSync(path.join(overrides,file.path))).digest('hex');
 if(sha===file.afterSha256)continue;
 file.previousAfterSha256s||=[];
 if(!file.previousAfterSha256s.includes(file.afterSha256))file.previousAfterSha256s.push(file.afterSha256);
 file.afterSha256=sha;
}
fs.writeFileSync(manifestPath,JSON.stringify(manifest,null,2)+'\n');
console.log(JSON.stringify({version:'director-native-overlay/v1',files:manifest.files.length,adapterSha256:crypto.createHash('sha256').update(source).digest('hex')}));
