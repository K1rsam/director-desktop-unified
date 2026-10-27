#!/usr/bin/env node
// Development-only: Node 24 transform. This file is not imported by the runtime CLI.
import {stripTypeScriptTypes} from 'node:module';
import {readFile,writeFile,realpath} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';

const hash=data=>createHash('sha256').update(data).digest('hex');
const output=dirname(fileURLToPath(import.meta.url));
const [option,sourceDirectory,...extra]=process.argv.slice(2);
if(option!=='--source-dir'||!sourceDirectory||extra.length)throw new Error('node24 scripts/intake/generate.mjs --source-dir /absolute/original-project');
if(!process.versions.node.startsWith('24.'))throw new Error('Generate with Node 24; later Node releases may not support mode:transform. Runtime CLI needs only the generated .mjs.');
const source=await realpath(resolve(sourceDirectory));
const rewrites=[{from:'../runtime/inline-performance.mjs',to:'../core/runtime/inline-performance.mjs'},
                {from:'./standard-script',to:'./standard-script.mjs'}];
const files=[];
for(const name of ['standard-script','standard-script-plan']){
 const raw=await readFile(join(source,'lib',name+'.ts'));
 let generated=stripTypeScriptTypes(raw.toString('utf8'),{mode:'transform'});
 const applied=[];
 for(const rewrite of rewrites){const from="'"+rewrite.from+"'";if(generated.includes(from)){generated=generated.replaceAll(from,"'"+rewrite.to+"'");applied.push(rewrite);}}
 await writeFile(join(output,name+'.mjs'),generated);
 files.push({upstream:'lib/'+name+'.ts',packaged:name+'.mjs',sourceSha256:hash(raw),generatedSha256:hash(generated),importRewrites:applied});
}
const inline=await readFile(join(source,'runtime/inline-performance.mjs'));
const packagedInline=await readFile(join(output,'../core/runtime/inline-performance.mjs'));
if(hash(inline)!==hash(packagedInline))throw new Error('Existing inline-performance.mjs differs from the original source snapshot; do not silently substitute.');
const manifest={version:'series-intake-v1',generator:{api:'node:module.stripTypeScriptTypes',mode:'transform',nodeVersion:process.version,
 command:'node24 scripts/intake/generate.mjs --source-dir /absolute/original-project',
 steps:['Read the two original UTF-8 TypeScript files without edits.','Run stripTypeScriptTypes(source,{mode:"transform"}); this handles constructor parameter properties.','Rewrite only the recorded local import specifiers.','Write generated .mjs and record original/generated SHA-256.']},
 files,runtimeDependency:{upstream:'runtime/inline-performance.mjs',packaged:'../core/runtime/inline-performance.mjs',sha256:hash(packagedInline)},
 runtimeCompilationRequired:false,semanticReview:'NOT_RUN'};
await writeFile(join(output,'provenance.json'),JSON.stringify(manifest,null,2)+'\n');
console.log(JSON.stringify({ok:true,nodeVersion:process.version,generated:files.map(f=>f.packaged),runtimeCompilationRequired:false}));
