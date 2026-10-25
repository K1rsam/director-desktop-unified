import {createHash} from 'node:crypto';

export const VERSION='performance-core-v4';
export const PLAN_VERSION=VERSION+'-plan';
const stable=value=>Array.isArray(value)?value.map(stable):value&&typeof value==='object'
  ?Object.fromEntries(Object.keys(value).sort().filter(k=>value[k]!==undefined).map(k=>[k,stable(value[k])])):value;
export const digest=value=>createHash('sha256').update(JSON.stringify(stable(value))).digest('hex');
export function fail(code,message){throw Object.assign(new Error(message),{code});}
