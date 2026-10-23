// Opt-in hidden WebView acceptance. The native shell uses an isolated offline
// home, disables all workers/accounts, and records the actual embedded frontend.
(async () => {
  if (window.__lingmoProbe) return; window.__lingmoProbe = true;
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const request = request => invoke('desktop_request', { request });
  const wait = async (condition, label) => { for(let i=0;i<300;i++){if(condition())return;await new Promise(resolve=>setTimeout(resolve,50));}throw Error(label); };
  const button = text => [...document.querySelectorAll('button')].find(b => b.textContent.trim() === text);
  const change = (element, value) => { Object.getOwnPropertyDescriptor(element.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(element,value);element.dispatchEvent(new Event('change',{bubbles:true}));element.dispatchEvent(new Event('input',{bubbles:true})); };
  try {
    await wait(()=>document.querySelector('.project-card'),'project view');
    let projects = await request({action:'projects.list'});
    let project = projects.find(p=>p.title==='灵墨原生导入验收');
    if(!project) throw Error('isolated Lingmo project not seeded');
    const pid=project.id, call = req=>request({...req,projectId:pid});
    const before = await call({action:'workspace.get'});
    const reopened = before.lingmoDocuments.length>0;
    const t0=performance.now();
    const preview=await call({action:'lingmo.preview',path:window.__lingmoVerificationInput});
    const expected={episodes:150,scenes:371,fragments:883,shots:5407,mentionOccurrences:4736,assets:518};
    if(JSON.stringify(Object.fromEntries(Object.keys(expected).map(k=>[k,preview.counts[k]])))!==JSON.stringify(expected))throw Error('full sample counts');
    const imported=await call({action:'lingmo.import',path:window.__lingmoVerificationInput,expectedSha256:preview.markdownSha256,previewToken:preview.previewToken});
    const importMs=performance.now()-t0;
    if(imported.jobs.length || JSON.stringify(imported.source)!==JSON.stringify(before.source))throw Error('import touched generation/source');
    if(imported.units.some(u=>u.origin==='imported-storyboard'&&(u.prompt||u.shots||u.mentions)))throw Error('workspace eagerly returned full drafts');
    // Refresh native React registry, enter the imported project, then render the
    // real Stage 4 component. No browser/foreground automation is involved.
    await wait(()=>[...document.querySelectorAll('.project-card')].some(c=>c.dataset.projectId===pid),'created project');
    [...document.querySelectorAll('.project-card')].find(c=>c.dataset.projectId===pid).querySelector('.project-card-open').click();
    await wait(()=>[...document.querySelectorAll('.stage-nav button')].some(b=>!b.disabled && b.textContent.includes('单元视频')),'stage navigation enabled after project load');
    [...document.querySelectorAll('.stage-nav button')].find(b=>b.textContent.includes('单元视频')).click();
    const displayStart=performance.now();
    await wait(()=>document.querySelectorAll('.lingmo-scene-assets .lingmo-asset-card').length && document.querySelectorAll('.lingmo-token').length,'first scene assets/tags');
    const firstDisplayMs=performance.now()-displayStart;
    // Exercise the same fresh-workspace replacement used by periodic polling.
    // Hidden idle windows pause the clock, so use the actual refresh button.
    const assetRegion=document.querySelector('.lingmo-scene-assets'), firstUnit=document.querySelector('.unit-card');
    const identityDetails=assetRegion.querySelector('details'); identityDetails.open=true;
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
      if(document.querySelector('.lingmo-scene-assets')!==assetRegion || document.querySelector('.unit-card')!==firstUnit || !identityDetails.open)throw Error('workspace refresh remounted the current scene');
    }
    observer.disconnect();
    if(sceneRemovals!==0)throw Error('unchanged workspace removed scene content');
    const doc=imported.lingmoDocuments.find(d=>d.id===preview.markdownSha256);
    const sceneSelect=()=>[...document.querySelectorAll('.lingmo-navigation select')][2];
    const episodeSelect=()=>[...document.querySelectorAll('.lingmo-navigation select')][1];
    if(sceneSelect().selectedOptions[0].textContent.indexOf('1-1')<0)throw Error('first scene');
    change(episodeSelect(),doc.episodes[1].id);
    await wait(()=>sceneSelect()?.options.length===doc.episodes[1].sceneIds.length && sceneSelect()?.value===doc.episodes[1].sceneIds[0],'episode 2 scenes');
    if(doc.episodes[1].sceneIds.length>1){change(sceneSelect(),doc.episodes[1].sceneIds[1]);await wait(()=>!document.querySelector('.lingmo-panel [role=status]')&&document.querySelectorAll('.lingmo-token').length,'scene 2 switch');}
    const last=await call({action:'lingmo.scene',documentId:doc.id,sceneId:doc.episodes[149].sceneIds[0]});
    const lastLocations=last.assets.filter(asset=>asset.lingmoType==='SC');
    if(lastLocations.length!==2)throw Error('last scene location count');
    const switchStart=performance.now();change(episodeSelect(),doc.episodes[149].id);
    await wait(()=>sceneSelect()?.options.length===1 && sceneSelect()?.selectedOptions[0]?.textContent.includes('150-1') && lastLocations.every(asset=>document.querySelector('.lingmo-scene-assets')?.textContent.includes(asset.name)),'episode 150 one scene two locations');
    const switchMs=performance.now()-switchStart;
    if(last.units.at(-1).duration!==4 || last.units.at(-1).actualDurationSeconds!==2.7)throw Error('decimal timeline');
    const final=await call({action:'workspace.get'});if(final.jobs.length)throw Error('unexpected generation');
    await invoke('startup_verification_report',{report:{ok:true,lingmoFullSample:true,saveReopenVerified:reopened,frontendRendered:true,ipcRoundTrip:true,counts:expected,markdownSha256:doc.id,importMs,firstDisplayMs,switchMs,lazySceneLoading:true,lastSceneTwoLocations:true,workspaceRefreshesVerified:workspaceReads,sceneRemovals,sceneNodesRetained:true,sourceUntouched:true,generationRequestsSent:0,generationJobs:0}});
  } catch(error) {await invoke('startup_verification_report',{report:{ok:false,error:String(error),assetCards:document.querySelectorAll('.lingmo-asset-card').length,assetTags:document.querySelectorAll('.lingmo-token').length,sceneSelectors:document.querySelectorAll('.lingmo-navigation select').length,sceneLoading:!!document.querySelector('.lingmo-panel [role=status]'),generationRequestsSent:0}});}
})();
