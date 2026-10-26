// Opt-in hidden WebView acceptance. The native shell uses an isolated offline
// home, disables all workers/accounts, and records the actual embedded frontend.
(async () => {
  if (window.__lingmoProbe) return; window.__lingmoProbe = true;
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const request = request => invoke('desktop_request', { request });
  const wait = async (condition, label) => { for(let i=0;i<300;i++){if(condition())return;await new Promise(resolve=>setTimeout(resolve,50));}throw Error(label); };
  const button = text => [...document.querySelectorAll('button')].find(b => b.textContent.trim() === text);
  const expectedStages=['剧本策划','分镜创作','提示词与图片','配图与视频'];
  const stageButton=name=>[...document.querySelectorAll('.stage-nav button')].find(b=>b.querySelector('strong')?.textContent===name);
  const change = (element, value) => { Object.getOwnPropertyDescriptor(element.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(element,value);element.dispatchEvent(new Event('change',{bubbles:true}));element.dispatchEvent(new Event('input',{bubbles:true})); };
  try {
    await wait(()=>document.querySelector('.project-card'),'project view');
    if(JSON.stringify([...document.querySelectorAll('.stage-nav button strong')].map(node=>node.textContent))!==JSON.stringify(expectedStages))throw Error('current four-stage navigation');
    const projectsBefore = await request({action:'projects.list'});
    const homeImport=document.querySelector('.lingmo-home-import button');
    if(!homeImport || [...document.querySelectorAll('.stage-nav button')].some(b=>!b.disabled))throw Error('home import requires a selected project');
    homeImport.click();await wait(()=>document.querySelector('dialog[open] .lingmo-import-dialog'),'home import dialog');
    document.querySelector('dialog[open] button[aria-label="关闭"]').click();
    const t0=performance.now();
    const preview=await request({action:'lingmo.preview_file',path:window.__lingmoVerificationInput});
    const expected={episodes:150,scenes:371,fragments:883,shots:5407,mentionOccurrences:4736,assets:518};
    if(JSON.stringify(Object.fromEntries(Object.keys(expected).map(k=>[k,preview.counts[k]])))!==JSON.stringify(expected))throw Error('full sample counts');
    const imported=await request({action:'lingmo.create_project',path:window.__lingmoVerificationInput,expectedSha256:preview.markdownSha256,previewToken:preview.previewToken});
    const pid=imported.project.id, call=req=>request({...req,projectId:pid});
    const reopened=projectsBefore.some(project=>project.id===pid);
    const savedPage=localStorage.getItem('director.project-page.v1:'+pid);
    const importMs=performance.now()-t0;
    if(imported.jobs.length || imported.source!==null)throw Error('import touched generation/source');
    if(imported.units.some(u=>u.origin==='imported-storyboard'&&(u.prompt||u.shots||u.mentions)))throw Error('workspace eagerly returned full drafts');
    // Refresh native React registry, enter the imported project, then render the
    // real Stage 4 component. No browser/foreground automation is involved.
    [...document.querySelectorAll('.sidebar-link')].find(b=>b.textContent.includes('所有项目')).click();
    await wait(()=>[...document.querySelectorAll('.project-card')].some(c=>c.dataset.projectId===pid),'created project');
    [...document.querySelectorAll('.project-card')].find(c=>c.dataset.projectId===pid).querySelector('.project-card-open').click();
    await wait(()=>stageButton('配图与视频') && !stageButton('配图与视频').disabled,'stage navigation enabled after project load');
    const expectedOpeningStage=savedPage==='images'?'提示词与图片':'配图与视频';
    await wait(()=>stageButton(expectedOpeningStage)?.classList.contains('selected'),'imported project preserves the actual saved work page');
    if(reopened && savedPage!=='images')throw Error('previous hidden run did not persist the image work page');
    stageButton('配图与视频').click();
    const displayStart=performance.now();
    await wait(()=>document.querySelectorAll('.lingmo-scene-assets .lingmo-asset-card').length && document.querySelectorAll('.lingmo-token').length,'first scene assets/tags');
    const firstDisplayMs=performance.now()-displayStart;
    button('本剧素材').click();
    await wait(()=>document.querySelector('dialog[open] .lingmo-materials'),'whole-document materials');
    if(document.querySelectorAll('.lingmo-materials-card').length!==80)throw Error('material list must paginate the full asset inventory');
    change(document.querySelector('[aria-label="素材类型"]'),'SE');
    await wait(()=>document.querySelectorAll('.lingmo-materials-card').length===0,'material type filter');
    document.querySelector('dialog[open] button[aria-label="关闭"]').click();
    let localFolder=null;
    if(window.__lingmoVerificationMedia){
      const baseline=imported.units.map(unit=>[unit.id,unit.version]);
      const folderPreview=await call({action:'lingmo.media.preview',documentId:preview.markdownSha256,directory:window.__lingmoVerificationMedia});
      if(folderPreview.counts.files!==518 || folderPreview.counts.matched!==518)throw Error('full folder exact matching');
      const folderResult=await call({action:'lingmo.media.commit',documentId:preview.markdownSha256,previewToken:folderPreview.previewToken,selections:folderPreview.rows.map(row=>({rowId:row.id,assetId:row.assetId,variantId:row.variantId,adopt:true,expectedAdoptedMediaId:row.expectedAdoptedMediaId??null}))});
      const report=folderResult.lingmoMediaImportReport;
      if(report.failed || report.placeholders!==518 || report.adopted!==0)throw Error('white placeholders must remain non-production candidates');
      if(folderResult.jobs.length || JSON.stringify(folderResult.units.map(unit=>[unit.id,unit.version]))!==JSON.stringify(baseline))throw Error('backfill changed frozen jobs or storyboards');
      localFolder={files:518,previewMilliseconds:folderPreview.milliseconds,importMilliseconds:report.milliseconds,placeholders:report.placeholders,adopted:report.adopted,failed:report.failed};
    }
    await new Promise(resolve=>setTimeout(resolve,800));
    // Exercise the same fresh-workspace replacement used by periodic polling.
    // Hidden idle windows pause the clock, so use the actual refresh button.
    const assetRegion=document.querySelector('.lingmo-scene-assets'), firstUnit=document.querySelector('.unit-card');
    assetRegion.querySelector('.lingmo-asset-card').click();
    await wait(()=>document.querySelector('dialog[open] .lingmo-asset-identity'),'asset manager');
    const assetDialog=document.querySelector('dialog[open]');
    const identityDetails=assetDialog.querySelector('.lingmo-asset-identity'); identityDetails.open=true;
    let workspaceReads=0, sceneRemovals=0;
    const observer=new MutationObserver(records=>{
      for(const record of records)for(const removed of record.removedNodes)if(removed===assetRegion || removed.contains?.(assetRegion))sceneRemovals++;
    });
    observer.observe(document.querySelector('.units-page'),{childList:true,subtree:true});
    for(let round=0;round<3;round++){
      await wait(()=>button('刷新')&&!button('刷新').disabled,'workspace refresh ready');button('刷新').click();
      await new Promise(resolve=>setTimeout(resolve,50));
      await wait(()=>!button('刷新').disabled && document.body.textContent.includes('已读取最新保存状态。'),'fresh workspace applied');
      await new Promise(resolve=>setTimeout(resolve,150));
      workspaceReads++;
      if(document.querySelector('.lingmo-scene-assets')!==assetRegion || document.querySelector('.unit-card')!==firstUnit || document.querySelector('dialog[open]')!==assetDialog || !identityDetails.open)throw Error('workspace refresh remounted the current scene');
    }
    observer.disconnect();
    if(sceneRemovals!==0)throw Error('unchanged workspace removed scene content');
    assetDialog.querySelector('button[aria-label="关闭"]').click();
    const doc=imported.lingmoDocuments.find(d=>d.id===preview.markdownSha256);
    const sceneSelect=()=>[...document.querySelectorAll('.lingmo-navigation select')][2];
    const episodeSelect=()=>[...document.querySelectorAll('.lingmo-navigation select')][1];
    if(sceneSelect().selectedOptions[0].textContent.indexOf('1-1')<0)throw Error('first scene');
    change(episodeSelect(),doc.episodes[1].id);
    await wait(()=>sceneSelect()?.options.length===doc.episodes[1].sceneIds.length && sceneSelect()?.value===doc.episodes[1].sceneIds[0],'episode 2 scenes');
    if(doc.episodes[1].sceneIds.length>1){change(sceneSelect(),doc.episodes[1].sceneIds[1]);await wait(()=>!document.querySelector('.lingmo-loading')&&document.querySelectorAll('.lingmo-token').length,'scene 2 switch');}
    const last=await call({action:'lingmo.scene',documentId:doc.id,sceneId:doc.episodes[149].sceneIds[0]});
    const lastLocations=last.assets.filter(asset=>asset.lingmoType==='SC');
    if(lastLocations.length!==2)throw Error('last scene location count');
    const switchStart=performance.now();change(episodeSelect(),doc.episodes[149].id);
    await wait(()=>sceneSelect()?.options.length===1 && sceneSelect()?.selectedOptions[0]?.textContent.includes('150-1') && lastLocations.every(asset=>document.querySelector('.lingmo-scene-assets')?.textContent.includes(asset.name)),'episode 150 one scene two locations');
    const switchMs=performance.now()-switchStart;
    if(last.units.at(-1).duration!==4 || last.units.at(-1).actualDurationSeconds!==2.7)throw Error('decimal timeline');
    // Exercise actual menu navigation and persisted project opening, without
    // calling image/text production or replacing React/IPC implementations.
    stageButton('提示词与图片').click();
    await wait(()=>document.querySelector('.asset-workspace') && localStorage.getItem('director.project-page.v1:'+pid)==='images','MD image work page');
    const writePrompts=document.querySelector('.asset-workspace .stage-intro button[data-execution-action="images.design"]');
    if(!writePrompts || writePrompts.textContent.trim()!=='编写全部提示词' || !writePrompts.disabled)throw Error('MD prompt entry or offline generation guard missing');
    if(!document.querySelector('.asset-workspace .stage-intro')?.textContent.includes('518 项正式图片'))throw Error('MD image scope was not projected into Stage 3');
    const reopenAt=async name=>{
      [...document.querySelectorAll('.sidebar-link')].find(b=>b.textContent.includes('所有项目')).click();
      await wait(()=>[...document.querySelectorAll('.project-card')].some(c=>c.dataset.projectId===pid),'saved MD project in library');
      [...document.querySelectorAll('.project-card')].find(c=>c.dataset.projectId===pid).querySelector('.project-card-open').click();
      await wait(()=>stageButton(name)?.classList.contains('selected'),'reopen at '+name);
    };
    await reopenAt('提示词与图片');
    stageButton('配图与视频').click();
    await wait(()=>localStorage.getItem('director.project-page.v1:'+pid)==='videos' && document.querySelector('.units-page'),'persist Stage 4 work page');
    await reopenAt('配图与视频');
    stageButton('提示词与图片').click();
    await wait(()=>document.querySelector('.asset-workspace') && localStorage.getItem('director.project-page.v1:'+pid)==='images','leave image page for process restart');
    const final=await call({action:'workspace.get'});if(final.jobs.length || final.source!==null)throw Error('unexpected generation or invented source');
    await invoke('startup_verification_report',{report:{ok:true,fourStageMenuVerified:true,mdStage3EntryVerified:true,mdStage4EntryVerified:true,projectWorkPageReopenVerified:true,saveReopenPageVerified:reopened&&savedPage==='images',homeImportWithoutProject:true,globalProjectImport:true,lingmoFullSample:true,saveReopenVerified:reopened,frontendRendered:true,ipcRoundTrip:true,counts:expected,markdownSha256:doc.id,importMs,firstDisplayMs,switchMs,lazySceneLoading:true,lastSceneTwoLocations:true,wholeDocumentMaterialsVerified:true,localFolder,workspaceRefreshesVerified:workspaceReads,sceneRemovals,sceneNodesRetained:true,sourceUntouched:true,generationRequestsSent:0,generationJobs:0}});
  } catch(error) {await invoke('startup_verification_report',{report:{ok:false,error:String(error),assetCards:document.querySelectorAll('.lingmo-asset-card').length,assetTags:document.querySelectorAll('.lingmo-token').length,sceneSelectors:document.querySelectorAll('.lingmo-navigation select').length,sceneLoading:!!document.querySelector('.lingmo-loading'),generationRequestsSent:0}});}
})();
