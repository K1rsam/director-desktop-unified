const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
function fixture({activity=true,native=false}={}){
 const nodes=[];
 function element(kind,index){return {children:[],parentElement:null,getAttribute:()=>'',matches(selector){if(kind==='user')return selector==='.bg-user-message';if(kind==='heading')return selector==='h4.sr-only';if(kind==='native')return selector.includes('[data-turn="assistant"]');return false;},closest(selector){if(selector==='[class~="group/agent-activity"]')return this.activity?this:null;if(kind==='native'&&selector.includes('[data-turn="assistant"]'))return this;return null;},querySelectorAll(selector){return selector==='[class*="MarkdownRoot"]'&&kind==='answer'?[{}]:[];},querySelector:()=>null,contains(node){return this.children.includes(node)},compareDocumentPosition(other){return index<other.index?4:2},index};}
 const user=element('user',1),progress=element(native?'native':'answer',2),answer=element('answer',3);
 for(const [parent,index]of [[progress,2],[answer,3]]){const heading=element('heading',index);heading.parentElement=parent;heading.activity=parent===progress&&activity;parent.children=[heading];nodes.push(heading);}
 nodes.unshift(user);if(native){progress.getAttribute=name=>name==='data-turn'?'assistant':name==='data-turn-id'?'original-native-id':'';nodes.splice(1,1,progress);}
 const context=vm.createContext({Node:{DOCUMENT_POSITION_PRECEDING:2,DOCUMENT_POSITION_FOLLOWING:4},document:{querySelectorAll:()=>nodes},ChatGptDomUtilities:{isPrimaryChatSurfaceElement:()=>true}});
 vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/content/turnDom.js'),'utf8'),context);
 return {dom:context.ChatGptTurnDom.createTurnDom(),user,progress,answer};
}
test('presentation-only agent activity does not become a second assistant image response',()=>{
 const f=fixture();assert.deepEqual([...f.dom.getTurnNodes()],[f.user,f.answer]);assert.deepEqual([...f.dom.getAssistantNodes()],[f.answer]);assert.equal(f.dom.key(f.answer),'');assert.equal(f.dom.key(f.user),'');
});
test('a second real assistant outside the activity panel remains visible to strict proof',()=>{
 const f=fixture({activity:false});assert.deepEqual([...f.dom.getTurnNodes()],[f.user,f.progress,f.answer]);assert.equal(f.dom.getAssistantNodes().length,2);
});
test('explicit native message identity is preserved even inside activity markup',()=>{
 const f=fixture({native:true});assert.equal(f.dom.getAssistantNodes().length,2);assert.equal(f.dom.key(f.progress),'original-native-id');
});
