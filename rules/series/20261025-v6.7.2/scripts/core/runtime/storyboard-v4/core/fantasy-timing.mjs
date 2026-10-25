export const FANTASY_TIMING_VERSION='fantasy-spans-v1';
const list=v=>Array.isArray(v)?v:[];
const positive=v=>Number.isFinite(v)&&v>0;
const same=(a,b)=>JSON.stringify(a)===JSON.stringify(b);

// Only arithmetic over explicit, valid shot intervals. Never interpret prose,
// guess an entrance/exit, clamp a range or change a shot/voice window.
export function fantasyTiming(record,draft){
 if(!Array.isArray(record?.screenSpans))return null;
 if(record.status==='not_realized')return record.screenSpans.length?null:{shotIds:[],screenSeconds:0};
 if(record.status!=='realized'||!record.screenSpans.length||!Array.isArray(record.unitIds)||!record.unitIds.length)return null;
 const shots=new Map(),units=new Set();let index=0;
 for(const unit of list(draft?.units)){
  if(typeof unit?.id!=='string'||units.has(unit.id))return null;units.add(unit.id);
  for(const shot of list(unit.shots)){
   if(typeof shot?.id!=='string'||shots.has(shot.id))return null;
   shots.set(shot.id,{unitId:unit.id,index:index++,duration:shot.duration});
  }
 }
 if(new Set(record.unitIds).size!==record.unitIds.length||record.unitIds.some(id=>!units.has(id)))return null;
 const shotIds=[];let seconds=0,last=-1,lastEnd=0;
 for(const span of record.screenSpans){
  const shot=shots.get(span?.shotId);
  if(!shot||!positive(shot.duration)||!record.unitIds.includes(shot.unitId)||shot.index<last)return null;
  let start=0,end=shot.duration;
  if(span.range!==null){
   if(!Array.isArray(span.range)||span.range.length!==2)return null;
   [start,end]=span.range;
   if(!Number.isFinite(start)||!Number.isFinite(end)||start<0||end<=start||end>shot.duration+1e-6)return null;
  }
  // A continuous shot can enter the same fantasy twice. Preserve the real gap
  // instead of forcing a cut or counting its enclosing interval as fantasy.
  if(shot.index===last&&start<lastEnd)return null;
  seconds+=end-start;if(shot.index!==last)shotIds.push(span.shotId);last=shot.index;lastEnd=end;
 }
 return {shotIds,screenSeconds:Number(seconds.toFixed(6))};
}

export function normalizeFantasyTiming(draft,version){
 const changes=[];
 if(version!==FANTASY_TIMING_VERSION)return changes;
 for(const record of list(draft?.fantasyRealization)){
  // Numeric legacy records may be retained by a delta reply. Do not invent
  // intervals for them or force a paid rewrite of unchanged material.
  if(!record||!Object.hasOwn(record,'screenSpans'))continue;
  const derived=fantasyTiming(record,draft);if(!derived)continue;
  for(const [field,to] of Object.entries({...derived,timingVersion:version}))if(!same(record[field],to)){
   changes.push({coordinate:record.candidateId,field,from:record[field],to,action:'derive',reason:'verified_shot_ranges'});record[field]=to;
  }
 }
 return changes;
}
