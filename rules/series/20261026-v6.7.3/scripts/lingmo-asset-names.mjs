import {spawnSync} from 'node:child_process';

const need=(ok,code,message)=>{if(!ok)throw Object.assign(new Error(message),{code});};
const prefix={CH:'角色',SC:'场景',PR:'道具'};
const cache=new Map();
const python=`import json,sys,unicodedata
values=json.load(sys.stdin)
result=[]
for value in values:
 safe=''.join(c for c in value if c.isalnum())
 result.append([safe,unicodedata.normalize('NFKC',safe).casefold()])
json.dump(result,sys.stdout)
`;

// Python's own Unicode database is authoritative. Batch callers once, then reuse
// the results for registry validation, tokens and filenames in this process.
export function sanitizeImportNames(values){
 need(Array.isArray(values)&&values.every(v=>typeof v==='string'),'ASSET_IMPORT_NAME','安全导入名须来自文字名称');
 const missing=[...new Set(values.filter(v=>!cache.has(v)))];
 if(missing.length){
  const result=spawnSync('python3',['-c',python],{input:JSON.stringify(missing),encoding:'utf8',maxBuffer:8*1024*1024,timeout:10000});
  need(!result.error&&result.status===0,'ASSET_IMPORT_PYTHON','安全命名需要可用的 Python 3 标准库');
  const data=JSON.parse(result.stdout);
  missing.forEach((value,i)=>{const [safe,fold]=data[i];cache.set(value,{safe,fold});cache.set(safe,{safe,fold});});
 }
 return values.map(value=>cache.get(value).safe);
}

export function sanitizeImportName(value){return sanitizeImportNames([value])[0];}

export function importAssetFields(asset){
 const name=asset?.importName,type=asset?.type,ordinal=asset?.placeholderOrdinal;
 need(typeof name==='string'&&name!==''&&name!=='无'&&sanitizeImportName(name)===name,'ASSET_IMPORT_NAME',`${asset?.id||asset?.key||'资产'} 的安全导入名须为非空 Python isalnum 字符串且不能是无`);
 need(Object.hasOwn(prefix,type),'ASSET_IMPORT_NAME','未知资产类型');
 need(Number.isSafeInteger(ordinal)&&ordinal>=0&&ordinal<=5700,'ASSET_PLACEHOLDER_ORDINAL',`${asset?.id||asset?.key||name} 占位编号须为 0–5700 的整数`);
 const filename=`${prefix[type]}-${name}.png`;
 need(Buffer.byteLength(filename,'utf8')<=255,'ASSET_IMPORT_NAME',`${asset?.id||asset?.key||name} 的图片文件名超过 255 字节`);
 return {name,type,token:name+type,filename,placeholderOrdinal:ordinal};
}

export function validateImportMappings(assets){
 need(Array.isArray(assets),'ASSET_IMPORT_NAME','缺少导入映射列表');
 sanitizeImportNames(assets.map(a=>a?.importName));
 const names=new Map(),files=new Map(),ordinals=new Set();
 for(const a of assets){
  const fields=importAssetFields(a),fold=cache.get(a.importName).fold,key=JSON.stringify([a.type,fold]),filename=`${prefix[a.type]}-${fold}.png`;
  need(!names.has(key)&&!files.has(filename),'ASSET_IMPORT_COLLISION',`${a.id||a.key||a.importName} 与 ${names.get(key)||files.get(filename)} 的安全名或图片文件名碰撞；须明确区分，不能合并ID`);
  need(!ordinals.has(fields.placeholderOrdinal),'ASSET_PLACEHOLDER_ORDINAL',`${a.id||a.key||a.importName} 的占位编号重复`);
  names.set(key,a.id||a.key||a.importName);files.set(filename,a.id||a.key||a.importName);ordinals.add(fields.placeholderOrdinal);
 }
 return true;
}
