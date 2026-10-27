import test from 'node:test';
import assert from 'node:assert/strict';
import {runPython,pythonCommand} from '../scripts/python-runtime.mjs';
test('actual Python runtime accepts UTF8 input and returns exact Unicode',()=>{
 const result=runPython(['-c','import sys; print(sys.stdin.read())'],{input:'角色面罩😀',encoding:'utf8'});
 assert.equal(result.status,0,result.stderr);
 assert.equal(result.stdout.trim(),'角色面罩😀');
});
test('invalid explicit interpreter fails instead of silently selecting another',()=>{
 const previous=process.env.STORYBOARD_PYTHON;
 try{
  process.env.STORYBOARD_PYTHON='__missing_desktop_storyboard_python__';
  assert.throws(()=>pythonCommand(),e=>e.code==='PYTHON_RUNTIME');
 }finally{
  if(previous===undefined)delete process.env.STORYBOARD_PYTHON;else process.env.STORYBOARD_PYTHON=previous;
 }
 assert.ok(pythonCommand().executable);
});
