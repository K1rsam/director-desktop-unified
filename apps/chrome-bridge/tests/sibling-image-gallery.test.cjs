const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const {parseHTML}=require('linkedom');
function fixture(change=()=>{}){
 const {document}=parseHTML('<html><body><main><div id="pair"><div class="bg-user-message">synthetic request</div><div class="group/agent-activity"><h4 class="sr-only">Activity</h4><div class="MarkdownRoot">Rendering</div></div><div id="gallery" data-testid="generated-image-gallery"><img src="https://chatgpt.com/backend-api/estuary/content?id=file-synthetic"></div><div id="answer"><h4 class="sr-only">Assistant</h4><div class="MarkdownRoot">synthetic final</div></div></div></main><aside id="sidebar"></aside></body></html>');
 change(document);
 const context=vm.createContext({document,Node:{DOCUMENT_POSITION_PRECEDING:2,DOCUMENT_POSITION_FOLLOWING:4},URL,location:{href:'https://chatgpt.com/c/synthetic'},ChatGptDomUtilities:{isPrimaryChatSurfaceElement:node=>Boolean(node.closest?.('main'))}});
 vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/content/turnDom.js'),'utf8'),context);
 const dom=context.ChatGptTurnDom.createTurnDom();return {document,context,dom,user:document.querySelector('.bg-user-message'),answer:document.querySelector('#answer'),gallery:document.querySelector('#gallery')};
}
test('presentation image before the final wrapper remains an image of the unique assistant, without merging the user or progress',()=>{
 const f=fixture();assert.deepEqual([...f.dom.getTurnNodes()],[f.user,f.answer]);
 assert.deepEqual([...f.dom.generatedImageScopes(f.answer)],[f.answer,f.gallery]);
 assert.deepEqual([...f.dom.generatedImageScopes(f.answer.querySelector('.MarkdownRoot'))],[f.answer,f.gallery]);
});
test('sibling gallery discovery rejects user attachments, sidebar images, later images and ambiguous histories',()=>{
 for(const change of [d=>d.querySelector('.bg-user-message').append(d.querySelector('#gallery')),d=>d.querySelector('#sidebar').append(d.querySelector('#gallery')),d=>d.querySelector('#pair').append(d.querySelector('#gallery')),d=>d.querySelector('#pair').insertAdjacentHTML('beforeend','<div class="bg-user-message">another request</div>'),d=>d.querySelector('#pair').insertAdjacentHTML('beforeend','<div><h4 class="sr-only">Another assistant</h4><div class="MarkdownRoot">another answer</div></div>'),d=>d.querySelector('#gallery').insertAdjacentHTML('afterend','<div data-testid="generated-image-gallery"><img></div>')]){
  const f=fixture(change);assert.deepEqual([...f.dom.generatedImageScopes(f.answer)],[f.answer]);
 }
});
test('actual artifact discovery and native proof use the same scoped gallery rather than the final text wrapper',()=>{
 const f=fixture();const image=f.gallery.querySelector('img');image.getBoundingClientRect=()=>({width:512,height:512});
 vm.runInContext(fs.readFileSync(path.join(__dirname,'../overrides/extension/content/artifactDom.js'),'utf8'),f.context);
 const api=f.context.ChatGptArtifactDom.createArtifactDom({DOM_PARSER:{extractFileLikeName:()=>'',classifyArtifactPhase:()=> 'READY',looksLikeFileDownloadUrl:()=>false},actionSelectorHint:()=>'',isUsableButton:()=>false,isVisible:()=>true,normalizeText:v=>String(v||'').trim(),simpleHash:()=> 'synthetic',visibleText:n=>n?.textContent||'',guessMime:()=> 'image/png',guessNameFromUrl:()=> 'synthetic.png'});
 const artifacts=api.collectArtifactsForAssistantNode(f.answer.querySelector('.MarkdownRoot'),{turnKey:'native-assistant'});
 assert.equal(artifacts.filter(a=>a.generatedImage).length,1);assert.equal(artifacts[0].sourceTurnKey,'native-assistant');
});
