// Development oracle only. Frozen JS is never loaded by the desktop runtime.
import fs from 'node:fs';
import {createHash} from 'node:crypto';
const frozen=new URL('../../../../rules/series-lite/20261027-v0.1.2/scripts/intake/standard-script-plan.mjs',import.meta.url);
const {prepareStandardEpisode}=await import(frozen);
const digest=createHash('sha256').update(fs.readFileSync(frozen)).digest('hex');
if(digest!=='7bfc1b027039d6b939118630dbc4a8fe516b3596de8ed5d187be877f94aec0ea')throw Error('oracle source drift');
const cases=[];
function add(name,text,options={},sourceId='SYNTHETIC-E1'){const input={text,sourceId,options};let c={name,input};try{c.output=prepareStandardEpisode(text,sourceId,options);}catch(e){c.error={name:e.name,code:e.code,fieldPath:e.fieldPath,sourceUnitId:e.sourceUnitId,message:e.message};}cases.push(c);}
const fixtures={
basic:'第1集\n1-1 日 外 庭院\n人物：甲、乙\n△甲走近。\n甲：你好。\n乙：走吧。\n',
empty:'',blank:' \t\r\n\n',emoji:'第1集\n人物：甲\n△天空😀晴朗。\n甲：你好😀。\n',crlf:'第1集\r\n人物：甲\r\n甲：原话。\r\n',indent:'\ufeff  第1集\n\t甲：原话。\n',
episode_qualifier:'第十二集（上）\n12-1：庭院\n甲：开始。',decimal_scene:'1.2 夜 外 街道\n甲：走吧。',named_scene:'SCENE: 庭院\n场景：餐厅\n甲：好。',metadata:'出场人物：甲、乙\n时间层：回忆\n天气：晴\n道具：杯子\n甲：原话。',
author:'【作者资料】\n设定：保留资料\n空白也保留\n\n第1集\n甲：开始。',guidance:'【镜头切】\n【镜头切：转到庭院】\n【二卡】\n导演处理：克制\n连续性备注：杯在左手\n情绪备注：迟疑\n甲：好。',typed:'【未知类型】\n甲：好。',end:'甲：结束。\n【全剧终】',omission:'……\n...\n．．．\n甲：原话。',
screen:'字幕：三年后\n弹幕（观众）：来了！\n屏幕文字: 任务完成\n甲：好。',empty_screen:'字幕：\n甲：好。',empty_action:'△ \t\n▲\n甲：好。',missing_speaker:'：原话\n（低声）：原话',
unknown_colon:'未知说明：这是资料\n甲：好。',unknown_plain:'这是一行动作。',unknown_cue:'无名者（低声）：原话。',unknown_plain_name:'无名者：原话。',
audio_roles:'旁白：天亮了。\n画外音：走吧。\n系统音：任务完成。',compact_voice:'李明OS：不能。\n李明 VO：来吧。',external_voice:'李明（低声）OS：不能。',latin_gap:'John Smith VO：Hello.\nMary：Yes.',latin_nogap:'JohnVO：原话。',
joint:'甲、乙：一起走。\n李明及王红：好。\n甲以及乙：来吧。',source_roles:'围观学生甲：走。\n前排男同学：好。\n众学生：来。\n三人：等等。\n系统窗口：任务奖励：金币。',incidental_roles:'联盟观察者：收到。\n其他三位老兵：好。\n遥远世界的无名者：疑问。',qualified:'愤怒的李明：走开。\nQ版李明：哈哈。\n李明Q版：好。',
cast_qualified:'人物：李明（少年）、王红（成年，长发）、John Smith\n李明（少年）：原话。\n李明少年：原话。\nJohn Smith：Hello.',cast_parenthetical_channel:'角色：李明（OS）、王红（VO）\n李明：原话。',
pre_cue:'甲（低声）：原话。',pre_channel:'甲（OS）：原话。',pre_cue_channel:'甲（低声,OS）：原话。',pre_channel_cue:'甲（VO，低声）：原话。',pre_channel_middle:'甲（低声,OS,咬牙）：原话。',pre_multi_channel:'甲（OS,VO）：原话。',unsupported_channel:'甲（O.S.）：原话。',bare_cue:'李明低声：原话。',bare_cue_compact:'李明低声OS：原话。',bare_cue_meta:'李明低声（咬牙）：原话。',bare_known:'人物：小鱼\n小鱼低声：原话。',bare_known_whole:'人物：小鱼低声\n小鱼低声：原话。',
post_cue:'甲：（低声）原话。',post_channel:'甲：（OS）原话。',post_compact_channel:'甲：（OS低声）原话。',post_cue_channel:'甲：（低声，VO）原话。',post_middle_channel:'甲：（低声，VO，咬牙）原话。',post_empty:'甲：（）原话。',post_nested:'甲：（低声（咬牙））原话。',post_quoted:'甲：（“低声”）原话。',post_colon:'甲：（提示：低声）原话。',post_double:'甲：（低声）（咬牙）原话。',post_unsupported_channel:'甲：（旁白）原话。',post_both_channel:'甲（OS）：（VO）原话。',post_both_cue:'甲（低声）：（咬牙）原话。',post_empty_body_action:'甲：（低头）\n乙：好。',post_empty_body_voice:'甲：（OS）\n乙：好。',
continuation:'甲：我们现在\n应该离开\n乙：好。',continuation_header:'人物：甲\n甲\n我们现在\n应该离开\n△乙出现。',continuation_blank:'甲：我们\n\n现在走吧\n乙：好。',continuation_finished:'甲：走吧。\n门缓缓打开。\n乙：好。',continuation_name:'人物：甲、乙\n甲：继续\n乙向门口走去\n甲：等等。',actor_action:'甲：走吧\n甲猛地转身。\n乙：好。',actor_speech:'甲：原话\n甲转身说道。',quoted_continuation:'甲：“我们\n乙：这还是引号内\n走吧。”\n乙：好。',unclosed_quote:'甲：“我们\n乙：还是引号内\n',closed_quote_follow:'甲：“原话”\n门打开\n乙：好。',possessive:"John：James' book is here.\nMary：It's mine.",escaped_quote:'甲：“他说\\”原话。”',
group:'甲：你好。乙：走吧。',group_space:'甲：你好 乙：走吧',group_no_boundary:'甲：你好乙：走吧',group_cue:'甲（低声）：你好。乙（OS）：走吧。',group_post:'甲：（低声）你好。乙：（VO）走吧。',group_latin:'John Smith：Hello. Mary Jane：Yes.',group_quote:'甲：“乙：原话。”乙：好。',group_unknown:'甲：你好。无名者：走吧。',group_named:'人物：小鱼、小猫\n小鱼：你好。小猫：走吧。',group_unclosed:'甲：“你好。乙：走吧。',
utterance_explain:'甲：方法很简单：立即离开。',utterance_system:'系统：任务目标：到达城门。',utterance_read:'甲（朗读）：条款一：必须离开。',utterance_silent:'甲（默读）：条款一：必须离开。',quoted_colon:'甲：他说：“离开。”',bracket_colon:'甲：【备注：原句】保留。',
inline_subtitle:'甲：我们【字幕：三年后】走吧。',inline_subtitle_two:'甲：【字幕：前一天】原话【SUBTITLE:Next】结束。',inline_subtitle_empty:'甲：原话【字幕：】',inline_subtitle_unclosed:'甲：原话【字幕：未闭合',inline_subtitle_ascii:'甲：原话[SUBTITLE:No]',inline_subtitle_quoted:'甲：“【字幕：字样】”。',group_subtitle:'甲：原话【字幕：三年后】。乙：走吧。',subtitle_only:'甲：【字幕：三年后】',
inline_performance:'甲：我们（低声）离开。',inline_v2:'甲：原来如此（恍然大悟）。',inline_protected:'甲：这是（说明，不是提示）原话。',inline_quote:'甲：“（低声）”这个词。',unicode_separators:'甲：原文\u2028不断开。',source_labels:'规则说明：保持原文\n导演处理低声：不可猜人\n甲：好。',number_roles:'1：走。\n学生A：好。\n甲：原话。',colon_only:'甲：\n乙：好。',bare_audio:'旁白\n天亮了。',quote_action:'“引用：这是动作。”',
};
for(const [name,text]of Object.entries(fixtures))add(name,text);
for(const name of ['basic','compact_voice','external_voice','post_cue','post_channel','joint','group','group_named','group_post','inline_subtitle','inline_performance','inline_v2','decimal_scene','episode_qualifier','guidance','screen','end','source_roles'])for(const [mode,options]of Object.entries({legacy:{syntax:'legacy'},inline_off:{inlinePerformance:false},v1:{inlinePerformanceVersion:'inline-performance-v1'}}))add(name+'_'+mode,fixtures[name],options);
add('known_names','无名者：原话。\n另一人：结束。',{knownSpeakers:['无名者','另一人']});
add('known_filtered',' 无名者：原话。',{knownSpeakers:[null,1,'','  无名者  ']});
for(const gap of ['\u00a0','\u3000','\ufeff','\t',' '])for(const position of ['pre','post'])add(`cue_whitespace_${position}_${gap.codePointAt(0)}`,position==='pre'?`甲（${gap}低声${gap}）：原话。`:`甲：（${gap}低声${gap}）原话。`);
for(const name of ['围观学生甲','学生A','学生a','台边女考官','联盟观察者','这是非常非常长的老兵','其他三位老兵','资料研究员','规则工作人员','愤怒的甲','愤怒的未知人','Q版甲','甲Q版','QQ版甲','Q版Q版甲','Q版规则','赵一','欧阳明','张四五六','Jane Doe','JANE','甲乙'])add(`identity_${name}`,`${name}：原话。`,{knownSpeakers:['甲']});
for(const cue of ['低声','咬牙','恍然大悟','恍然大悟的样子','突然憋笑','声音微微发抖','笑容瞬间凝固','气得满脸通红','再次提示','未知动作','低声，咬牙'])add(`compact_cue_${cue}`,`李明${cue}OS：原话。`);
for(const line of ['甲： （低声）原话。','甲：（低声） （咬牙）原话。','甲（）：原话。','甲（低声）（咬牙）：原话。','甲（低声） VO：原话。','甲VO（低声）：原话。','甲（低声）OS','甲（低声）os：原话。','甲（低声, os）：原话。','甲：（低声,O.S.）原话。','甲：（OS 咬牙）原话。','甲：（os咬牙）原话。','甲：（VO低声,咬牙）原话。','甲：（低声,VO咬牙）原话。','甲：（ 低声，咬牙 ）原话。','甲： (OS) 原话。','甲：原话。 乙：继续。 丙：结束。','甲：原话，乙：继续','甲：原话…乙：继续','甲：“原话。”乙：继续','甲：[备注：引述]原话。','甲：I said "John: yes".','甲：James\' story.','甲：（低声）','未知人（OS）：原话。','未知人：（VO）原话。','未知人（低声）：原话。'])add('layout_'+line,line);
for(const text of ['甲：开始\n提示：未知\n乙：结束。','甲：\n\n原话\n\n乙：结束。','甲：开始\n他说：“继续。”\n△结束。','甲：原话【字幕：字样】\n继续\n乙：结束。','甲：“跨行\n\n继续。”\n乙：结束。','甲：原话。\n\n继续\n乙：结束。','甲：\n△只有动作。','人物：小鱼（少年）、小鱼（成年）\n小鱼（少年）：原话。\n小鱼（成年）：结束。','场景：\n甲：原话。','【作者资料】\n第2集（下）\n甲：开始。','甲：原话\n……\n乙：结束。','甲：原话\n字幕：三年后\n乙：结束。'])add('continuation_edge_'+text,text);
add('nul_and_surrogate_pair','人物：甲😀\n甲😀：原话。\n甲：🧑‍🚀原话。',{knownSpeakers:['甲😀']});
add('invalid_performance',fixtures.basic,{inlinePerformanceVersion:'unknown'});
add('null_performance',fixtures.basic,{inlinePerformanceVersion:null});
add('postColonCue_false',fixtures.post_cue,{postColonCue:false});
add('invalid_text',null);add('empty_source_id',fixtures.basic,{},'');
fs.writeFileSync(new URL('series_lite_intake_plan_golden.json',import.meta.url),JSON.stringify({sourceCommit:'6dea8d6c57dde38f04a4d71f05f4c039004dd4a9',sourceSHA256:digest,cases},null,2)+'\n');
console.log(JSON.stringify({cases:cases.length,errors:cases.filter(c=>c.error).length,diagnostic:cases.filter(c=>c.output?.diagnostics.length).length,compiled:cases.filter(c=>c.output?.compiled).length}));
