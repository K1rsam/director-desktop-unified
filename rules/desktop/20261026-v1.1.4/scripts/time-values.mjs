export const SECOND_TICKS=1e9;
export function timeTicks(value,coordinate,code='TIME_PRECISION',positive=false){
 const need=(ok,c,message)=>{if(!ok)throw Object.assign(new Error(message),{code:c});};
 need(Number.isFinite(value)&&(positive?value>0:value>=0),code,`${coordinate} 时间须为${positive?'正':'非负'}有限数`);
 const scaled=value*SECOND_TICKS,ticks=Math.round(scaled),noise=Number.EPSILON*Math.max(1,Math.abs(scaled))*4;
 need(Number.isSafeInteger(ticks)&&!(value>0&&ticks===0)&&Math.abs(scaled-ticks)<=noise,'TIME_PRECISION',`${coordinate} 超出9位小数可表达精度，返回局部处理，不能静默改时长或声窗`);
 return ticks;
}
export const timeText=v=>v.toFixed(9).replace(/\.?0+$/,'');
export const containerSeconds=ticks=>Math.max(4,Math.floor((ticks+SECOND_TICKS-1)/SECOND_TICKS));
