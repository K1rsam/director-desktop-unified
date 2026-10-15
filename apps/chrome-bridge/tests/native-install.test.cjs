const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),os=require('node:os'),path=require('node:path'),{spawnSync}=require('node:child_process');
const script=path.resolve(__dirname,'../scripts/install-native-host.ps1');
function fixture(t){
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'director-native-install-'));
 t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const component=path.join(root,'component & copy'),extension=path.join(component,'extension'),edge=path.join(root,'Edge User Data'),chrome=path.join(root,'Chrome User Data');
 fs.mkdirSync(extension,{recursive:true});fs.mkdirSync(path.join(edge,'Default'),{recursive:true});
 fs.writeFileSync(path.join(component,'package.json'),JSON.stringify({name:'chatgpt-browser-bridge-node',version:'6.4.0'}));
 const prefs=path.join(edge,'Default','Secure Preferences');
 fs.writeFileSync(prefs,JSON.stringify({extensions:{settings:{['a'.repeat(32)]:{path:extension},['b'.repeat(32)]:{path:path.join(root,'other-extension')}}},privateFixture:'未改变的中文配置'}));
 const binary=path.join(root,'fixture.exe');fs.writeFileSync(binary,'synthetic-not-executable');
 const install=path.join(root,'host install');
 const run=(extra=[],installRoot=install)=>spawnSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',script,'-PlanOnly','-HostBinary',binary,'-ComponentDirectory',component,'-InstallRoot',installRoot,'-EdgeUserDataRoot',edge,'-ChromeUserDataRoot',chrome,...extra],{encoding:'utf8',timeout:15000,windowsHide:true});
 return {root,prefs,component,extension,install,run};
}
test('native install plan binds only the registered exact extension and leaves browser files untouched',{skip:process.platform!=='win32'},t=>{
 const f=fixture(t),before=fs.readFileSync(f.prefs);let r=f.run(),registeredManifest,beforeManifest;
 if(r.status!==0&&r.stderr.includes('NATIVE_INSTALL_FOREIGN_REGISTRATION')){
  // A real pre-existing registration must stay protected. The second call is
  // still PlanOnly and never creates or updates the registered destination.
  const read=spawnSync('powershell.exe',['-NoProfile','-Command',"(Get-Item -LiteralPath 'HKCU:\\Software\\Microsoft\\Edge\\NativeMessagingHosts\\app.director.browser_bridge').GetValue('')"],{encoding:'utf8',windowsHide:true,timeout:10000});
  assert.equal(read.status,0);registeredManifest=read.stdout.trim();assert.ok(path.isAbsolute(registeredManifest));
  beforeManifest=fs.readFileSync(registeredManifest);r=f.run([],path.dirname(registeredManifest));
 }
 assert.equal(r.status,0,r.stderr);
 const p=JSON.parse(r.stdout);assert.deepEqual(p.manifest.allowed_origins,['chrome-extension://'+'a'.repeat(32)+'/']);
 assert.equal(p.config.serverUrl,'http://127.0.0.1:53179');assert.equal(p.registrations.length,1);assert.equal(p.registrations[0].browser,'Edge');assert.equal(p.policiesModified,false);
 assert.deepEqual(fs.readFileSync(f.prefs),before);assert.equal(fs.existsSync(f.install),false);assert.ok(!JSON.stringify(p).includes('privateFixture'));
 if(registeredManifest)assert.deepEqual(fs.readFileSync(registeredManifest),beforeManifest);
});
test('malformed browser JSON reports a fixed code without echoing profile data',{skip:process.platform!=='win32'},t=>{
 const f=fixture(t);fs.writeFileSync(f.prefs,'{"privateMarker":"synthetic-private-value",broken');const r=f.run();assert.notEqual(r.status,0);assert.match(r.stderr,/NATIVE_INSTALL_PREFERENCES_INVALID/);assert.ok(!r.stderr.includes('synthetic-private-value'));assert.equal(fs.existsSync(f.install),false);
});
test('foreign directory and malformed extension id never produce an install plan',{skip:process.platform!=='win32'},t=>{
 const f=fixture(t);for(const [id,directory]of [['b'.repeat(32),f.extension+'-different'],['z'.repeat(32),f.extension]]){
  fs.writeFileSync(f.prefs,JSON.stringify({extensions:{settings:{[id]:{path:directory}}}}));const r=f.run();assert.notEqual(r.status,0);assert.match(r.stderr,/NATIVE_INSTALL_(REGISTERED_EXTENSION_NOT_FOUND|EXTENSION_ID_INVALID)/);assert.equal(fs.existsSync(f.install),false);
 }
});
test('linked install target is rejected before staging or profile modifications',{skip:process.platform!=='win32'},t=>{
 const f=fixture(t),target=path.join(f.root,'outside');fs.mkdirSync(target);fs.symlinkSync(target,f.install,'junction');const before=fs.readFileSync(f.prefs),r=f.run();assert.notEqual(r.status,0);assert.match(r.stderr,/NATIVE_INSTALL_LINKED_PATH/);assert.deepEqual(fs.readFileSync(f.prefs),before);assert.deepEqual(fs.readdirSync(target),[]);
});
