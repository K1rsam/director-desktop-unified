export const VISUAL_SUBJECTS_VERSION='visual-subjects-v1';
export const VISUAL_SUBJECT_DOMAINS=['subjective','prospective'];
const key=subject=>JSON.stringify([subject.name,subject.domain]);

// This manifest lists visual-only subjects; it never grants presence, ability or knowledge.
export function validVisualSubjects(subjects){
 return Array.isArray(subjects)&&subjects.every(s=>s&&typeof s==='object'&&!Array.isArray(s)&&Object.keys(s).every(k=>k==='name'||k==='domain')&&typeof s.name==='string'&&s.name.trim()&&VISUAL_SUBJECT_DOMAINS.includes(s.domain))&&new Set(subjects.map(key)).size===subjects.length;
}

export function mergeVisualSubjects(units){
 const seen=new Set(),subjects=[];
 for(const u of units)for(const subject of u.visualSubjects||[]){
  const id=key(subject);if(seen.has(id))continue;
  seen.add(id);subjects.push(structuredClone(subject));
 }
 return subjects;
}
