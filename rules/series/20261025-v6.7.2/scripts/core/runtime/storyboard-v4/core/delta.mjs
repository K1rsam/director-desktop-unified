import {VERSION,digest,fail} from './shared.mjs';

export const DELTA_VERSION='performance-core-v4-delta';
export const baseDraftHash=draft=>digest({schemaVersion:draft.schemaVersion,units:draft.units,fantasyRealization:draft.fantasyRealization});

export function applyDelta(base,delta){
 if(delta?.schemaVersion!==DELTA_VERSION||!Array.isArray(delta.retainedUnitIds)||!Array.isArray(delta.replacements))fail('DELTA_SHAPE','差量响应缺少保留单元或完整替换组');
 if(delta.baseDraftHash!==baseDraftHash(base))fail('DELTA_STALE','差量基稿与实际现稿不一致');
 const oldIds=base.units.map(u=>u.id),oldSet=new Set(oldIds),claimed=new Set(),newIds=new Set();
 const claim=id=>{if(!oldSet.has(id)||claimed.has(id))fail('DELTA_PARTITION','旧单元须精确分区覆盖，不得重复或越界');claimed.add(id);};
 for(const id of delta.retainedUnitIds)claim(id);
 const replacements=new Map();
 for(const group of delta.replacements){
  if(!Array.isArray(group?.replaces)||!group.replaces.length||!Array.isArray(group.units)||!group.units.length||!Array.isArray(group.fantasyRealization))fail('DELTA_SHAPE','替换组须有旧单元、完整新单元和幻想记录数组');
  const positions=group.replaces.map(id=>oldIds.indexOf(id));
  if(positions.some((position,i)=>position<0||(i>0&&position!==positions[i-1]+1)))fail('DELTA_ORDER','替换组旧单元须按现稿顺序连续排列；跨越单元须纳入同组');
  for(const id of group.replaces){claim(id);replacements.set(id,group);}
  for(const unit of group.units){if(typeof unit?.id!=='string'||!unit.id||newIds.has(unit.id)||(oldSet.has(unit.id)&&!group.replaces.includes(unit.id)))fail('DELTA_UNIT_ID','新单元ID为空、重复或复用其他组旧ID');newIds.add(unit.id);}
 }
 if(claimed.size!==oldIds.length)fail('DELTA_PARTITION','旧单元不得遗漏');
 if(delta.retainedUnitIds.some(id=>newIds.has(id)))fail('DELTA_UNIT_ID','新单元ID不得撞保留单元');
 const oldFantasy=base.fantasyRealization||[];
 for(const fantasy of oldFantasy){const touched=new Set((fantasy.unitIds||[]).map(id=>replacements.get(id)).filter(Boolean));if(touched.size>1||(touched.size===1&&(fantasy.unitIds||[]).some(id=>replacements.get(id)!==[...touched][0])))fail('DELTA_FANTASY_GROUP','关联幻想必须随完整替换组一起处理');}
 const units=[],seenGroups=new Set();
 for(const unit of base.units){const group=replacements.get(unit.id);if(!group)units.push(unit);else if(!seenGroups.has(group)){seenGroups.add(group);units.push(...group.units);}}
 const retained=new Set(delta.retainedUnitIds);
 const fantasyRealization=[...oldFantasy.filter(f=>(f.unitIds||[]).every(id=>retained.has(id))),...delta.replacements.flatMap(g=>g.fantasyRealization)];
 for(const group of delta.replacements){const allowed=new Set(group.units.map(u=>u.id));if(group.fantasyRealization.some(f=>!Array.isArray(f?.unitIds)||f.unitIds.some(id=>!allowed.has(id))))fail('DELTA_FANTASY_GROUP','替换幻想只能引用本组完整新单元');}
 return {schemaVersion:VERSION,units,fantasyRealization,...(delta.revisionNotes?{revisionNotes:delta.revisionNotes}:{}),...(delta.unresolved?{unresolved:delta.unresolved}:{})};
}
