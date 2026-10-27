import {spawnSync} from 'node:child_process';

let cached;
// An explicit interpreter is authoritative; never silently replace a bad override.
export function pythonCommand(){
 const configured=process.env.STORYBOARD_PYTHON||'';
 if(cached?.configured===configured)return cached.command;
 const candidates=configured?[[configured,[]]]:[['python3',[]],['python',[]],['py',['-3']]];
 for(const [executable,prefix] of candidates){
  const probe=spawnSync(executable,[...prefix,'-X','utf8','-c','import sys; print("storyboard-python-ok" if sys.version_info >= (3,9) else "too-old")'],
   {encoding:'utf8',timeout:8000,windowsHide:true});
  if(!probe.error&&probe.status===0&&probe.stdout.trim()==='storyboard-python-ok'){
   const command={executable,args:[...prefix,'-X','utf8']};cached={configured,command};return command;
  }
 }
 throw Object.assign(new Error('需要Python 3.9+；可将STORYBOARD_PYTHON设置为实际解释器绝对路径'),{code:'PYTHON_RUNTIME'});
}
export function runPython(args,options={}){
 const command=pythonCommand();
 return spawnSync(command.executable,[...command.args,...args],{...options,windowsHide:true});
}
