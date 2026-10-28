import {VERSION,PLAN_VERSION} from './shared.mjs';
import {DELTA_VERSION} from './delta.mjs';
import {FANTASY_TIMING_VERSION} from './fantasy-timing.mjs';
import {VISUAL_SUBJECTS_VERSION,VISUAL_SUBJECT_DOMAINS} from './visual-subjects.mjs';

export const OUTPUT_SCHEMA_VERSION='structured-v2';
export const UNIT_DIRECTION_VERSION='unit-direction-v1';
const string={type:'string'},number={type:'number'};
const object=properties=>({type:'object',properties,required:Object.keys(properties),additionalProperties:false});
const array=(items,limits={})=>({type:'array',items,...limits});
const choice=values=>values.length?{type:'string',enum:[...new Set(values)]}:string;
const references=values=>array(choice(values),values.length?{}:{maxItems:0});
const range=()=>array(number,{minItems:2,maxItems:2});

// Shape and known identifiers only; the existing domain validators retain semantic checks.
export function buildOutputSchema({stage,finishMode,source,settings,selection,previousDraft,baseDraftHash,scenarioKeys,plan,outputSchemaVersion='structured-v1',unitDirectionVersion,fantasyTimingVersion,visualSubjectsVersion}){
 const eventFirst=outputSchemaVersion==='structured-v2';
 const emotion=()=>object({subject:string,contextTone:string,audienceEffect:string,change:string,landing:string,retain:array(string)});
 const boundary=kinds=>object({kind:choice(kinds),reason:string});
 const sourceIds=source.sourceRows.map(r=>r.id),contextIds=[...new Set([...sourceIds,...source.contextRows.map(r=>r.id)])];
 if(stage==='plan')return object({
  schemaVersion:choice([PLAN_VERSION]),emotionArc:string,
  ...(eventFirst?{unitPackingPolicy:choice(['event-first-v1']),events:array(object({id:string,sourceRefs:references(sourceIds),sceneKey:string,boundaryBefore:boundary(['episode_start','continuous','new_event','time_space_jump']),emotionContract:emotion()}))}:{}),
  entryFacts:array(object({id:string,subject:string,statement:string,sourceRefs:references(contextIds),evidenceMode:choice(['source_rows'])})),
  workPackages:array(object({id:string,...(eventFirst?{eventId:string}:{}),sourceRefs:references(sourceIds),emotionContract:emotion(),entryFactIds:array(string),contextRefs:references(contextIds),dependsOn:array(string),scenarioKeys:references(scenarioKeys)})),
  provisionalUnits:array(object({id:string,packageId:string,sourceRefs:references(sourceIds),secondsRange:range()})),
  fantasyCandidates:array(object({id:string,anchorRefs:references(sourceIds),subject:string,emotionPurpose:string,recommended:{type:'boolean'},proposal:string,screenSecondsRange:range(),extraSecondsRange:range()}),settings.fantasyEnabled?{}:{maxItems:0})
 });
 const dialogueIds=source.dialogue.map(q=>q.id),candidateIds=selection.selectedIds;
 const dialoguePart=object({lineId:choice(dialogueIds),text:string,presentation:choice(['画内','画外','VO','OS','系统音','画外转画内','画内转画外']),delivery:string,voiceStart:number,voiceEnd:number});
 const shot=object({id:string,duration:number,camera:string,picture:string,lighting:string,sound:string,dialogueParts:array(dialoguePart,dialogueIds.length?{}:{maxItems:0}),transition:string,sourceRefs:references(sourceIds),contextRefs:references(contextIds)});
 const units=array(object({id:string,...(eventFirst?{eventId:choice((plan?.events||[]).map(e=>e.id)),boundaryBefore:boundary(['episode_start','continuous','capacity','new_event','time_space_jump'])}:{}),title:string,...(unitDirectionVersion===UNIT_DIRECTION_VERSION?{unitDirection:string}:{}),intent:object({subject:string,change:string,landing:string,retain:array(string)}),scene:string,props:array(string),participants:array(string),...(visualSubjectsVersion===VISUAL_SUBJECTS_VERSION?{visualSubjects:array(object({name:string,domain:choice(VISUAL_SUBJECT_DOMAINS)}))}:{}),lighting:string,initialState:string,exitState:string,shots:array(shot)}));
 const fantasyRealization=array(object({candidateId:choice(candidateIds),unitIds:array(string),...(fantasyTimingVersion===FANTASY_TIMING_VERSION?{screenSpans:array(object({shotId:string,range:{anyOf:[{type:'null'},range()]}}))}:{shotIds:array(string),screenSeconds:number}),entry:string,exit:string,domain:string,preservedFunction:string,status:choice(['realized','not_realized'])}),candidateIds.length?{}:{maxItems:0});
 const revision={revisionNotes:array(object({coordinate:string,sourceRefs:references(sourceIds),change:string,preservedFunction:string})),unresolved:array(object({coordinate:string,problem:string}))};
 if(stage==='finish'&&finishMode==='delta'){
  const oldIds=previousDraft.units.map(u=>u.id);
  return object({schemaVersion:choice([DELTA_VERSION]),baseDraftHash:choice([baseDraftHash]),retainedUnitIds:references(oldIds),replacements:array(object({replaces:references(oldIds),units,fantasyRealization})),...revision});
 }
 return object({schemaVersion:choice([VERSION]),units,fantasyRealization,...(stage==='finish'?revision:{})});
}
