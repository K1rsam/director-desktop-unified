#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(all(not(debug_assertions), not(feature = "custom-protocol")))]
compile_error!("Release desktop builds require custom-protocol to embed the frontend. Use npm run desktop:build -- --no-bundle, or enable desktop,custom-protocol.");

use director_core::{Engine, startup_diagnostics::{Stage as DiagnosticStage, StartupDiagnostics}};
use std::sync::Arc;
use tauri::Manager;

// Explicit, offline release verification. It uses its own home and never opens a production project.
#[derive(Clone)]
struct StartupVerification {
    output: std::path::PathBuf,
    home: std::path::PathBuf,
    finished: Arc<std::sync::atomic::AtomicBool>,
    development_server_at_start: bool,
    diagnostics: Arc<StartupDiagnostics>,
}

fn development_server_available() -> bool {
    ["127.0.0.1:1420", "[::1]:1420"].iter().any(|address| {
        std::net::TcpStream::connect_timeout(
            &address.parse().unwrap(),
            std::time::Duration::from_millis(150),
        )
        .is_ok()
    })
}

#[tauri::command]
fn startup_verification_report(
    app: tauri::AppHandle,
    verification: tauri::State<'_, Option<StartupVerification>>,
    report: serde_json::Value,
) -> Result<(), String> {
    let v = verification.as_ref().ok_or("未启用启动验证")?;
    v.diagnostics.mark(DiagnosticStage::ReportReceived);
    let mut report = report;
    let background_window = app.get_webview_window("main").is_some_and(|window|
        window.is_visible().ok()==Some(false) && window.is_focused().ok()==Some(false));
    report["backgroundWindowVerified"]=serde_json::json!(background_window);
    if !background_window {
        report["ok"]=serde_json::json!(false);
        report["error"]=serde_json::json!("Offline startup verification must remain hidden and unfocused");
    }
    let frontend_url = app
        .get_webview_window("main")
        .and_then(|window| window.url().ok());
    let embedded_frontend = frontend_url.as_ref().is_some_and(|url| {
        url.port().is_none()
            && matches!(
                (url.scheme(), url.host_str()),
                ("http" | "https", Some("tauri.localhost")) | ("tauri", Some("localhost"))
            )
    });
    let development_server_at_finish = development_server_available();
    let standalone = cfg!(feature = "custom-protocol")
        && !cfg!(dev)
        && embedded_frontend
        && !v.development_server_at_start
        && !development_server_at_finish;
    if !standalone {
        report["ok"] = serde_json::json!(false);
        report["error"] = serde_json::json!("Standalone verification requires the production protocol and no development server on port 1420");
    }
    report["developmentMode"] = serde_json::json!(cfg!(dev));
    report["customProtocol"] = serde_json::json!(cfg!(feature = "custom-protocol"));
    report["embeddedFrontend"] = serde_json::json!(embedded_frontend);
    report["frontendUrl"] = serde_json::json!(frontend_url.map(|url| url.to_string()));
    report["developmentServerAtStart"] = serde_json::json!(v.development_server_at_start);
    report["developmentServerAtFinish"] = serde_json::json!(development_server_at_finish);
    report["standalone"] = serde_json::json!(standalone);
    report["platform"] = serde_json::json!(std::env::consts::OS);
    let unvirtualized = director_core::windows_runtime::verify_shared_directory(&v.home).is_ok();
    report["unvirtualizedRuntime"] = serde_json::json!(unvirtualized);
    report["sharedProjectRegistry"] = serde_json::json!(unvirtualized);
    if !unvirtualized {
        report["ok"] = serde_json::json!(false);
        report["error"] = serde_json::json!("Desktop startup still inherited a packaged host filesystem");
    }
    report["diagnosticPid"] = serde_json::json!(std::process::id());
    report["diagnosticSchema"] = serde_json::json!("startup-diagnostics/v1");
    report["home"] = serde_json::json!(v.home);
    let success = report["ok"] == true;
    director_core::store::atomic_json(&v.output, &report)?;
    v.diagnostics.mark(DiagnosticStage::ReportPersisted);
    v.finished.store(true, std::sync::atomic::Ordering::SeqCst);
    app.exit(if success { 0 } else { 2 });
    Ok(())
}

#[tauri::command]
fn startup_verification_progress(
    verification: tauri::State<'_, Option<StartupVerification>>,
    checkpoint: String,
) -> Result<(), String> {
    let v = verification.as_ref().ok_or("未启用启动验证")?;
    v.diagnostics.frontend(&checkpoint).map_err(str::to_owned)
}

const STARTUP_PROBE: &str = r#"
(async () => {
  if (window.__directorStartupProbe) return;
  window.__directorStartupProbe = true;
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const progress = async checkpoint => {
    try { await invoke('startup_verification_progress', {checkpoint}); } catch { /* best-effort fixed metadata only */ }
  };
  try {
    await progress('started');
    for (let attempt = 0; attempt < 100; attempt++) {
      if (document.querySelector('.local-status')?.textContent.includes('本地核心已连接') && document.querySelector('.project-card')) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    if (!document.querySelector('.project-card')) throw new Error('React project view did not render');
    const projects = await invoke('desktop_request', {request:{action:'projects.list'}});
    // A second launch against this same isolated home verifies persisted data.
    // The first launch leaves precisely these two projects after lifecycle tests.
    if (projects.length === 2 && projects.some(project => project.title === '行内项目一') && projects.some(project => project.title === '行内项目二')) {
      const global = await invoke('desktop_request',{request:{action:'settings.global.get'}});
      if (global.schemaVersion !== 'app-settings/v1' || global.settings.textModel !== 'startup-global-model') throw new Error('Software settings did not survive process restart');
      const savedProject = projects.find(project => project.title === '行内项目二');
      const saved = await invoke('desktop_request', {request:{action:'workspace.get',projectId:savedProject.id}});
      if (saved.source.text !== '第1集\n场景：门口\n甲：保存后重开。\n' || saved.jobs.length || saved.assets.length !== 1 || saved.units.length !== 1 || saved.media.length !== 1) throw new Error('Saved workspace did not survive process restart');
      const asset = saved.assets[0], media = saved.media[0], unit = saved.units[0];
      if (asset.id !== 'reopen-asset' || asset.adoptedMediaId !== media.id || unit.id !== 'reopen-unit' || unit.assetRefs[0]?.assetId !== asset.id) throw new Error('Saved asset, image or unit identity changed after restart');
      const image = new Image();
      await new Promise((resolve,reject)=>{image.onload=resolve;image.onerror=()=>reject(new Error('Saved media failed to load after restart'));image.src=window.__TAURI_INTERNALS__.convertFileSrc(media.path);});
      if(image.naturalWidth!==32 || image.naturalHeight!==24) throw new Error('Saved media dimensions changed after restart');
      const card=[...document.querySelectorAll('.project-card')].find(card=>card.dataset.projectId===savedProject.id);
      if(!card) throw new Error('Saved project did not appear in the native library');
      card.querySelector('.project-card-open').click();
      for(let attempt=0;attempt<100 && !document.querySelector('.production-source-document')?.textContent.includes('甲：保存后重开。');attempt++) await new Promise(resolve=>setTimeout(resolve,50));
      if(!document.querySelector('.production-source-document')?.textContent.includes('甲：保存后重开。')) throw new Error('Saved source did not render after restart');
      await progress('completed');
      await invoke('startup_verification_report',{report:{ok:true,saveReopenVerified:true,frontendRendered:true,ipcRoundTrip:true,sqliteRead:true,assetProtocol:true,projectId:savedProject.id,mediaSha256:media.sha256,projectCount:projects.length,generationRequestsSent:0,generationJobs:0}});
      return;
    }
    if (projects.length !== 1) throw new Error('Isolated project registry did not round trip');
    const workspace = await invoke('desktop_request', {request:{action:'workspace.get', projectId:projects[0].id}});
    if (!workspace.source.text || workspace.jobs.length || workspace.media.length !== 1) throw new Error('SQLite workspace did not round trip');
    await progress('project_round_trip');
    const media = workspace.media[0];
    const image = new Image();
    await new Promise((resolve, reject) => {
      image.onload = resolve; image.onerror = () => reject(new Error('Asset protocol image did not load'));
      image.src = window.__TAURI_INTERNALS__.convertFileSrc(media.path);
    });
    if (image.naturalWidth !== 32 || image.naturalHeight !== 24) throw new Error('Image dimensions changed');
    await progress('asset_protocol');
    document.querySelector('.project-card-open').click();
    for (let attempt=0; attempt<100 && !document.querySelector('.stage-nav button:not(:disabled)'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    const foundationButton=[...document.querySelectorAll('.stage-nav button')].find(button=>button.textContent.includes('剧本与基础'));
    foundationButton.click();
    for (let attempt=0; attempt<100 && !document.querySelector('.stage-one-workbench'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    if (!document.querySelector('[aria-label="集场目录"]') || !document.querySelector('[aria-label="制作基础内容"]') || !document.querySelector('.production-source-document')?.textContent.includes('甲：走吧。')) throw new Error('Stage 1 locked source workbench did not render');
    if (!document.querySelector('.source-structure-summary')?.textContent.includes('已识别 1 集 / 1 场') || document.querySelector('dialog[open]')) throw new Error('Stage 1 source structure summary is missing or requires an unsolicited dialog');
    await progress('stage_one');
    const structureButton=[...document.querySelectorAll('.stage-one-workbench button')].find(button=>button.textContent==='查看分集结构');
    if (!structureButton) throw new Error('Source structure review entry is missing');
    structureButton.click();
    for (let attempt=0; attempt<100 && !document.querySelector('[aria-label="原文识别结构"]'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    const backgroundButton=document.querySelector('.source-context-link');
    if (!backgroundButton) throw new Error('Source background directory is missing');
    backgroundButton.click();
    for (let attempt=0; attempt<100 && document.querySelector('.production-reading-heading h3')?.textContent!=='背景资料'; attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    if (!document.querySelector('.production-source-document')?.textContent.includes('【人物关系】') || document.querySelector('.production-source-document')?.textContent.includes('甲：走吧。')) throw new Error('Background readback mixes production body or lost original context');
    await progress('source_review');
    const storyboardButton=[...document.querySelectorAll('.stage-nav button')].find(button=>button.textContent.includes('分镜提示词'));
    if (!storyboardButton) throw new Error('Storyboard navigation is missing');
    storyboardButton.click();
    for (let attempt=0; attempt<100; attempt++) {
      if ([...document.querySelectorAll('.stage-three-workbench button')].some(button=>button.textContent.includes('生成分镜提示词') && button.disabled) && !document.querySelector('.video-production-panel')) break;
      await new Promise(resolve=>setTimeout(resolve,50));
    }
    const creationButton=[...document.querySelectorAll('button')].find(button=>button.textContent.includes('生成分镜提示词'));
    if (!creationButton || !creationButton.disabled || !document.querySelector('.production-range-label')?.textContent.includes('创作范围') || document.querySelector('.video-production-panel')) throw new Error('Offline Stage 3 must render its range but disable generation');
    const policy = await invoke('desktop_request',{request:{action:'getExecutionPolicy'}});
    if(policy.liveExecutionAllowed !== false || policy.stage3CreativePackStatus !== 'inactive') throw new Error('Offline execution policy is not enforced');
    let blockedGeneration = false;
    try { await invoke('desktop_request',{request:{action:'storyboard.run',projectId:projects[0].id}}); } catch(error) { blockedGeneration = String(error).includes('OFFLINE_EXECUTION_BLOCKED'); }
    if(!blockedGeneration) throw new Error('Raw IPC bypassed the offline generation gate');
    if (!document.querySelector('.text-execution-summary')?.textContent.includes('Codex CLI')) throw new Error('Stage text executor identity did not render');
    await progress('stage_three');
    const settingsButton=[...document.querySelectorAll('.sidebar-link')].find(button=>button.textContent.includes('软件设置'));
    if (!settingsButton) throw new Error('Settings navigation is missing');
    settingsButton.click();
    for (let attempt=0; attempt<100 && !document.querySelector('.dreamina-account-card'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    const accountCard=document.querySelector('.dreamina-account-card');
    if (!accountCard || accountCard.getAttribute('aria-label') !== '即梦生成 CLI' || !accountCard.textContent.includes('OAuth 登录')) throw new Error('OAuth account panel did not render');
    const cliCard=document.querySelector('.text-execution-settings');
    if (!cliCard || !cliCard.textContent.includes('OpenAI') || !cliCard.textContent.includes('SDK 与 CLI 共用 OpenAI 账户')) throw new Error('Unified OpenAI settings did not render');
    const software = await invoke('desktop_request',{request:{action:'settings.global.get'}});
    if(software.schemaVersion !== 'app-settings/v1') throw new Error('Software settings require an unexpected project');
    const savedSoftware = await invoke('desktop_request',{request:{action:'settings.global.save',expectedRevision:software.revision,settings:{textModel:'startup-global-model'}}});
    if(savedSoftware.settings.textModel !== 'startup-global-model' || savedSoftware.revision !== software.revision+1) throw new Error('Software settings did not save');
    await progress('settings');
    // Software settings is global and deliberately hides project navigation.
    // Return to the library before exercising the selected project's controls.
    const libraryButton=[...document.querySelectorAll('.sidebar-link')].find(button=>button.textContent.includes('所有项目'));
    if (!libraryButton) throw new Error('Library navigation is missing');
    libraryButton.click();
    for (let attempt=0; attempt<100 && !document.querySelector('.stage-nav button'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    // Reversible mutations in the isolated diagnostic project exercise the real
    // Tauri IPC and SQLite lifecycle. No provider worker is running in this mode.
    const pid=projects[0].id;
    const call=request=>invoke('desktop_request',{request:{projectId:pid,...request}});
    await call({action:'assets.save',asset:{id:'probe-asset',variantId:'base',name:'离线测试资产',kind:'character',sourceRefs:[],prompt:'隔离测试',applicability:{episodes:[],scenes:[]}}});
    await call({action:'media.adopt',assetId:'probe-asset',variantId:'base',mediaId:media.id});
    const before=await call({action:'assets.get',assetId:'probe-asset'});
    const usage=await call({action:'assets.usage',assetId:before.id});
    if (usage.units.length || usage.imageCount!==1 || usage.version!==before.version) throw new Error('Asset usage inspection did not round trip');
    const removed=await call({action:'assets.delete',assetId:before.id,expectedVersion:before.version});
    if (removed.assets.some(asset=>asset.id===before.id) || !removed.deletedAssets.some(asset=>asset.id===before.id && asset.adoptedMediaId===media.id)) throw new Error('Soft delete did not preserve the image');
    const deleted=removed.deletedAssets.find(asset=>asset.id===before.id);
    const restored=await call({action:'assets.restore',assetId:before.id,expectedVersion:deleted.version});
    if (!restored.assets.some(asset=>asset.id===before.id && asset.adoptedMediaId===media.id) || restored.deletedAssets.some(asset=>asset.id===before.id)) throw new Error('Restoration changed asset or image identity');
    const imageButton=[...document.querySelectorAll('.stage-nav button')].find(button=>button.textContent.includes('图片资产'));
    if (!imageButton) throw new Error('Image navigation is missing after leaving software settings');
    imageButton.click();
    for (let attempt=0; attempt<100 && !document.querySelector('.asset-workspace'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    const refreshButton=[...document.querySelectorAll('.topbar button')].find(button=>button.textContent.includes('刷新'));
    refreshButton.click();
    for (let attempt=0; attempt<100 && !document.querySelector('.asset-management-actions'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    const management=document.querySelector('.asset-management-actions');
    if (!management || !management.textContent.includes('编辑资产') || !management.textContent.includes('删除资产')) throw new Error('Asset lifecycle controls did not render in the native client');
    const editAsset=[...management.querySelectorAll('button')].find(button=>button.textContent==='编辑资产');
    for (let attempt=0; attempt<100 && editAsset.disabled; attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    if (editAsset.disabled) throw new Error('Full asset detail never became editable');
    editAsset.click();
    for (let attempt=0; attempt<100 && !document.querySelector('.asset-management-form'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    if (!document.querySelector('dialog[open] .asset-management-form')) throw new Error('Asset edit dialog did not open');
    document.querySelector('dialog[open] .modal-header button').click();
    await progress('asset_lifecycle');
    const episode=workspace.source.episodes[0].id, scene=workspace.source.scenes[0].id, row=workspace.source.rows.find(row=>row.kind==='body' && row.episodeId===episode && row.sceneId===scene).id;
    await call({action:'units.save',unit:{id:'probe-unit',episodeId:episode,sceneId:scene,sourceRefs:[row],title:'离线配图验证',prompt:'推门进入 {{asset:probe-asset:base}}',duration:12,assetRefs:[{assetId:before.id,variantId:'base',role:'人物',order:1}]}});
    const videoButton=[...document.querySelectorAll('.stage-nav button')].find(button=>button.textContent.includes('单元视频'));
    videoButton.click();
    for (let attempt=0; attempt<100 && !document.querySelector('.units-page'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    refreshButton.click();
    let adjust;
    for (let attempt=0; attempt<100; attempt++) {
      adjust=[...document.querySelectorAll('.unit-materials button')].find(button=>button.textContent==='调整配图');
      if (adjust && !adjust.disabled) break;
      await new Promise(resolve=>setTimeout(resolve,50));
    }
    if (!adjust || adjust.disabled) throw new Error('Stage 4 material adjustment did not render');
    adjust.click();
    for (let attempt=0; attempt<100 && !document.querySelector('.unit-asset-edit-row'); attempt++) await new Promise(resolve=>setTimeout(resolve,50));
    if (!document.querySelector('dialog[open] .unit-asset-edit-row') || !document.querySelector('[aria-label="配图 1 资产"]')) throw new Error('Stage 4 material editor did not open');
    const finalWorkspace=await call({action:'workspace.get'});
    if (finalWorkspace.jobs.length) throw new Error('Offline asset lifecycle probe unexpectedly created a generation task');
    document.querySelector('dialog[open] .modal-header button').click();
    await progress('unit_editor');
    const waitFor=async (predicate,message)=>{
      for(let attempt=0; attempt<150; attempt++) { if(predicate()) return; await new Promise(resolve=>setTimeout(resolve,50)); }
      throw new Error(message);
    };
    const projectCard=()=>[...document.querySelectorAll('.project-card')].find(card=>card.dataset.projectId===pid);
    const tab=name=>[...document.querySelectorAll('.project-library-tabs button')].find(button=>button.textContent.includes(name));
    const cardAction=name=>[...projectCard()?.querySelectorAll('.project-card-actions button')||[]].find(button=>button.textContent===name || button.getAttribute('aria-label')?.startsWith(name+'：'));
    const confirmAction=async name=>{
      await waitFor(()=>document.querySelector('dialog[open] .project-lifecycle-dialog'),'Project lifecycle confirmation did not render');
      const confirm=[...document.querySelectorAll('dialog[open] .project-lifecycle-dialog button')].find(button=>button.textContent===name);
      if(!confirm || confirm.disabled) throw new Error('Project lifecycle confirmation is not enabled: '+name);
      confirm.click();
      await waitFor(()=>!document.querySelector('dialog[open]'),'Project lifecycle confirmation did not finish: '+name);
    };
    [...document.querySelectorAll('.sidebar-link')].find(button=>button.textContent.includes('所有项目')).click();
    await waitFor(()=>projectCard() && cardAction('归档项目'),'Active project management did not render');
    let prematureDeleteRejected=false;
    try { await call({action:'projects.delete',expectedVersion:0}); } catch { prematureDeleteRejected=true; }
    if(!prematureDeleteRejected) throw new Error('Unarchived project was deletable through IPC');
    cardAction('归档项目').click();
    await confirmAction('归档项目');
    tab('已归档').click();
    await waitFor(()=>cardAction('恢复项目') && cardAction('删除项目'),'Archived project management did not render');
    const archived=await invoke('desktop_request',{request:{action:'projects.list'}});
    if(!archived.find(project=>project.id===pid)?.archivedAt || document.querySelector('.project-card-open') || [...document.querySelectorAll('.stage-nav button')].some(button=>!button.disabled)) throw new Error('Archived project remained open or archive metadata was lost');
    cardAction('恢复项目').click();
    await waitFor(()=>!projectCard(),'Restored project remained in the archive list');
    await waitFor(()=>tab('进行中') && !tab('进行中').disabled,'Project restore did not finish refreshing the library');
    tab('进行中').click();
    await waitFor(()=>cardAction('归档项目'),'Restored project did not return to the active list');
    const restoredProject=(await invoke('desktop_request',{request:{action:'projects.list'}})).find(project=>project.id===pid);
    if(restoredProject.archivedAt) throw new Error('Restore did not clear the persisted archive state');
    const restoredWorkspace=await call({action:'workspace.get'});
    if(restoredWorkspace.source.sha256!==workspace.source.sha256 || !restoredWorkspace.media.some(item=>item.id===media.id && item.sha256===media.sha256)) throw new Error('Project restoration changed source or media identity');
    cardAction('归档项目').click();
    await confirmAction('归档项目');
    tab('已归档').click();
    await waitFor(()=>cardAction('删除项目'),'Restored project could not be archived again');
    cardAction('删除项目').click();
    await waitFor(()=>document.querySelector('dialog[open] .project-lifecycle-dialog'),'Project delete dialog did not render');
    const deleteText=document.querySelector('dialog[open] .project-lifecycle-dialog').textContent;
    if(!deleteText.includes('保留') || !deleteText.includes('目录')) throw new Error('Project deletion failed to disclose file retention');
    await confirmAction('删除项目');
    await waitFor(()=>!projectCard(),'Deleted project remained in the project library');
    const deletedProjects=await invoke('desktop_request',{request:{action:'projects.list'}});
    if(deletedProjects.some(project=>project.id===pid)) throw new Error('Deleted project remained registered');
    await progress('project_lifecycle');
    const createInline=async (name,useEnter)=>{
      await waitFor(()=>document.querySelector('.inline-project-create input') && !document.querySelector('.inline-project-create input').disabled,'Inline project entry is not ready');
      const form=document.querySelector('.inline-project-create'), input=form.querySelector('input');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,name);
      input.dispatchEvent(new Event('input',{bubbles:true}));
      await waitFor(()=>!form.querySelector('button[type="submit"]').disabled,'Inline project name did not enable submission');
      if(useEnter) form.requestSubmit(); else form.querySelector('button[type="submit"]').click();
      await waitFor(()=>document.querySelector('.stage-one-workbench') && document.querySelector('.sidebar-project strong')?.textContent===name.trim(),'Inline creation did not enter the new project Stage 1');
      if(document.querySelector('.inline-project-create')) throw new Error('Project creation entry remained in Stage 1');
      if(document.querySelector('dialog[open]')) throw new Error('Inline creation opened an unsolicited directory or project dialog');
      const projects=await invoke('desktop_request',{request:{action:'projects.list'}});
      const created=projects.find(project=>project.title===name.trim());
      if(!created) throw new Error('Inline project was not persisted');
      const createdWorkspace=await invoke('desktop_request',{request:{action:'workspace.get',projectId:created.id}});
      if(createdWorkspace.jobs.length || createdWorkspace.assets.length || createdWorkspace.source?.text) throw new Error('New project inherited another project data');
      return created;
    };
    const libraryCreated=await createInline('  行内项目一  ',false);
    [...document.querySelectorAll('.sidebar-link')].find(button=>button.textContent.includes('所有项目')).click();
    await waitFor(()=>document.querySelector('.inline-project-create input') && !document.querySelector('.inline-project-create input').disabled,'Returning to the project library did not restore the creation entry');
    if(document.querySelector('.inline-project-create input').value!=='') throw new Error('Library project name did not reset after creation');
    const secondLibraryCreated=await createInline('行内项目二',true);
    const parent=path=>path.replace(/\\/g,'/').split('/').slice(0,-1).join('/');
    if(libraryCreated.id===secondLibraryCreated.id || parent(libraryCreated.rootPath)!==parent(workspace.project.rootPath) || parent(secondLibraryCreated.rootPath)!==parent(workspace.project.rootPath)) throw new Error('Inline projects did not use independent directories under the same default root');
    const finalProjects=await invoke('desktop_request',{request:{action:'projects.list'}});
    if(finalProjects.length!==2 || document.querySelector('.inline-project-create')) throw new Error('Inline creation left duplicate projects or leaked the creation entry into Stage 1');
    const saveCall=request=>invoke('desktop_request',{request:{projectId:secondLibraryCreated.id,...request}});
    await saveCall({action:'source.import',text:'第1集\n场景：门口\n甲：保存后重开。\n'});
    await saveCall({action:'assets.save',asset:{id:'reopen-asset',variantId:'base',name:'重开测试资产',kind:'character',sourceRefs:[],prompt:'隔离验证',applicability:{episodes:[],scenes:[]}}});
    await saveCall({action:'media.import',path:media.path,kind:'image',assetId:'reopen-asset',variantId:'base'});
    const saved=await saveCall({action:'workspace.get'});
    if(saved.media.length!==1) throw new Error('Reopen fixture media was not saved');
    await saveCall({action:'media.adopt',assetId:'reopen-asset',variantId:'base',mediaId:saved.media[0].id});
    const savedEpisode=saved.source.episodes[0].id,savedScene=saved.source.scenes[0].id,savedRow=saved.source.rows.find(row=>row.kind==='body' && row.episodeId===savedEpisode && row.sceneId===savedScene).id;
    await saveCall({action:'units.save',unit:{id:'reopen-unit',episodeId:savedEpisode,sceneId:savedScene,sourceRefs:[savedRow],title:'保存后重开验证',prompt:'{{asset:reopen-asset:base}} 保存',duration:5,assetRefs:[{assetId:'reopen-asset',variantId:'base',role:'人物',order:1}]}});
    await progress('inline_create');
    await progress('completed');
    await invoke('startup_verification_report', {report:{ok:true, frontendRendered:true, ipcRoundTrip:true, sqliteRead:true, assetProtocol:true, accountPanelRendered:true, stage1LockedSourceRendered:true, sourceStructureReviewRendered:true, backgroundSourceRendered:true, stage3OfflineCreationBlocked:true, rawIpcGenerationBlocked:true, textCliSettingsRendered:true, textExecutionSummaryRendered:true, assetUsageRoundTrip:true, assetDeleteRestoreRoundTrip:true, imageRetainedAfterRestore:true, nativeAssetEditorOpened:true, nativeUnitAssetEditorOpened:true, projectArchiveRestoreDeleteUiRoundTrip:true, unarchivedProjectDeleteRejected:true, archivedWorkspaceClosed:true, sourceAndMediaRetainedAfterRestore:true, projectFileRetentionDisclosed:true, projectRemovedFromRegistry:true, libraryInlineCreateRoundTrip:true, libraryInlineEnterCreateRoundTrip:true, stageOneCreationEntryAbsent:true, sharedDefaultRoot:true, createdProjectIds:[libraryCreated.id,secondLibraryCreated.id], projectRoot:workspace.project.rootPath, originalMediaPath:media.path, rules:workspace.rules.map(rule=>({id:rule.id,version:rule.version})), generationRequestsSent:0, projectId:pid, mediaSha256:media.sha256, generationJobs:finalWorkspace.jobs.length, width:innerWidth, height:innerHeight}});
  } catch (error) {
    await progress('failed');
    await invoke('startup_verification_report', {report:{ok:false,error:String(error),pageTitle:document.querySelector('.topbar h1')?.textContent,assetControlsRendered:!!document.querySelector('.asset-management-actions'),unitControlsRendered:!!document.querySelector('.unit-reference-grid')}});
  }
})();
"#;

#[tauri::command]
async fn desktop_request(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Arc<Engine>>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let core = engine.inner().clone();
    let paths = core.clone();
    let result = tauri::async_runtime::spawn_blocking(move || core.dispatch(request))
        .await
        .map_err(|e| e.to_string())?;
    if let Ok(projects) = paths.project_list() {
        for p in projects {
            if let Some(path) = p["rootPath"].as_str() {
                for scope in director_core::platform::media_scopes(std::path::Path::new(path))? {
                    app.asset_protocol_scope().allow_directory(scope, true).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    result
}
fn main() {
    if director_core::text_sdk::run_runtime_wrapper().expect("SDK 执行子进程校验失败") {
        return;
    }
    // Leave packaged-host file redirection before opening application state or CLI workers.
    if director_core::windows_runtime::ensure_unvirtualized().expect("无法切换到共享桌面项目环境") {
        return;
    }
    let args: Vec<_> = std::env::args_os().collect();
    if let Some(index) = args.iter().position(|arg| arg == "--maintain-task-records") {
        let valid = args.iter().skip(1).enumerate().all(|(offset,arg)| {
            offset + 1 == index || offset + 1 == index + 1 || arg == "--unvirtualized-desktop"
        });
        let Some(path) = args.get(index + 1).map(std::path::PathBuf::from).filter(|path|path.is_absolute()) else {
            std::process::exit(64);
        };
        if !valid { std::process::exit(64); }
        let code = director_core::engine::task_record_maintenance::run_file(&path).unwrap_or(2);
        std::process::exit(code);
    }
    if let Some(index) = args.iter().position(|arg| arg == "--background-chrome-images") {
        let valid = args.iter().skip(1).enumerate().all(|(offset,arg)| {
            offset + 1 == index || offset + 1 == index + 1 || arg == "--unvirtualized-desktop"
        });
        let Some(path) = args.get(index + 1).map(std::path::PathBuf::from).filter(|path|path.is_absolute()) else {
            std::process::exit(64);
        };
        if !valid { std::process::exit(64); }
        let code = director_core::engine::background_chrome_images::run_file(&path).unwrap_or(2);
        std::process::exit(code);
    }
    let verification = args.iter().position(|a| a == "--verify-startup").map(|i| {
        let output = args.get(i + 1).map(std::path::PathBuf::from).unwrap_or_else(|| {
            eprintln!("--verify-startup requires an absolute report file path");
            std::process::exit(64);
        });
        let diagnostics = StartupDiagnostics::start(&output).unwrap_or_else(|error| {
            // No requested path, argument values or environment are printed.
            eprintln!("Startup diagnostics initialization failed: {:?}", error.kind());
            std::process::exit(64);
        });
        diagnostics.install_panic_hook();
        diagnostics.mark(DiagnosticStage::ArgumentsAccepted);
        StartupVerification {
            diagnostics,
            home: output.parent().unwrap().join("startup-verification-home"),
            output,
            finished: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            development_server_at_start: development_server_available(),
        }
    });
    let acceptance_grant = args.iter().position(|a| a == "--native-cli-acceptance").map(|i| {
        assert!(verification.is_none(), "启动烟测不能同时启用真实文本验收");
        let path=std::path::PathBuf::from(args.get(i+1).expect("--native-cli-acceptance requires a grant path"));
        director_core::native_acceptance::AcceptanceGrant::from_file(&path).expect("验收授权文件不合法")
    });
    let setup_verification = verification.clone();
    let page_verification = verification.clone();
    let run_verification = verification.clone();
    let build_verification = verification.clone();
    let closing = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let force_exit = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut context=tauri::generate_context!();
    if verification.is_some() {
        // Offline smoke tests exercise the real WebView/IPC with an isolated
        // home, without opening a foreground window over the user's work.
        for window in &mut context.config_mut().app.windows {window.visible=false;window.focus=false;}
    }
    if let Some(v) = &verification { v.diagnostics.mark(DiagnosticStage::DesktopBuildStarted); }
    tauri::Builder::default()
        .manage(verification)
        .setup(move |app| {
            if let Some(v) = &setup_verification { v.diagnostics.mark(DiagnosticStage::SetupEntered); }
            let home = match &setup_verification { Some(v) => v.home.clone(), None => app.path().app_data_dir()? };
            director_core::windows_runtime::verify_shared_directory(&home).map_err(std::io::Error::other)?;
            // Lifetime OS lease also excludes the explicit image-only background runner.
            // Held in Tauri state until every worker and final receipt has drained.
            let execution_lease = director_core::engine::background_chrome_images::HomeExecutionLease::acquire(&home).map_err(std::io::Error::other)?;
            app.manage(execution_lease);
            let saved_acceptance_grant = home.join("native-cli-acceptance.json").try_exists()?;
            // Diagnostic projects have no channel account configured. Disable live-account
            // lookup so automatic Stage 4 preflight fails locally without touching a user session.
            let engine = match &acceptance_grant {
                Some(grant) => Engine::with_native_acceptance(home,grant.clone()),
                None if setup_verification.is_some() => Engine::new_offline_verification(home),
                None => Engine::new_standard(home),
            }.map_err(std::io::Error::other)?;
            let resource_root=director_core::platform::native_resource_root(app.path().resource_dir()?).map_err(std::io::Error::other)?;
            if cfg!(windows) && setup_verification.is_none() && acceptance_grant.is_none() && !saved_acceptance_grant {
                director_core::text_sdk::enable_native_execution(resource_root.clone()).map_err(std::io::Error::other)?;
            }
            let engine=engine.with_native_resource_root(resource_root).map_err(std::io::Error::other)?;
            if let Some(v) = &setup_verification {
                v.diagnostics.mark(DiagnosticStage::EngineReady);
                if engine.project_list().map_err(std::io::Error::other)?.is_empty() {
                    let project = engine.dispatch(serde_json::json!({"action":"projects.create","title":"桌面启动离线验证"})).map_err(std::io::Error::other)?;
                    let pid = project["id"].as_str().unwrap();
                    engine.dispatch(serde_json::json!({"action":"source.import","projectId":pid,"text":"【故事简介】\n甲与旧友重逢。\n【人物关系】\n甲：主角。\n第1集\n场景：门口\n甲：走吧。\n"})).map_err(std::io::Error::other)?;
                    let png = v.home.join("startup-reference.png");
                    image::RgbImage::from_pixel(32, 24, image::Rgb([42, 73, 104])).save(&png)?;
                    engine.store(pid).map_err(std::io::Error::other)?.import_media(&png, "image", None, None).map_err(std::io::Error::other)?;
                }
                v.diagnostics.mark(DiagnosticStage::FixtureSeeded);
                let handle = app.handle().clone();
                let v = v.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    if !v.finished.load(std::sync::atomic::Ordering::SeqCst) {
                        v.diagnostics.mark(DiagnosticStage::WatchdogTimeout);
                        if director_core::store::atomic_json(&v.output, &serde_json::json!({"ok":false,"error":"Desktop startup probe timed out","diagnosticPid":std::process::id(),"diagnosticSchema":"startup-diagnostics/v1"})).is_ok() {
                            v.diagnostics.mark(DiagnosticStage::ReportPersisted);
                        }
                        handle.exit(3);
                    }
                });
            }
            for p in engine.project_list().map_err(std::io::Error::other)? {
                if let Some(path) = p["rootPath"].as_str() {
                    for scope in director_core::platform::media_scopes(std::path::Path::new(path)).map_err(std::io::Error::other)? {
                        app.asset_protocol_scope().allow_directory(scope, true)?;
                    }
                }
            }
            if let Some(v) = &setup_verification { v.diagnostics.mark(DiagnosticStage::MediaScopeReady); }
            if setup_verification.is_none() {
                engine.start();
            }
            app.manage(engine);
            if let Some(v) = &setup_verification { v.diagnostics.mark(DiagnosticStage::SetupComplete); }
            Ok(())
        })
        .on_page_load(move |webview, payload| {
            if let Some(v) = &page_verification {
                if payload.event() == tauri::webview::PageLoadEvent::Finished {
                    v.diagnostics.mark(DiagnosticStage::PageLoaded);
                    v.diagnostics.mark(if webview.eval(STARTUP_PROBE).is_ok() {
                        DiagnosticStage::ProbeInjected
                    } else { DiagnosticStage::ProbeInjectionFailed });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![desktop_request, startup_verification_report, startup_verification_progress])
        .build(context)
        .inspect_err(|_| { if let Some(v) = &build_verification { v.diagnostics.mark(DiagnosticStage::DesktopBuildFailed); } })
        .expect("无法启动制作工作台")
        .run(move |app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if let Some(v) = &run_verification { v.diagnostics.mark(DiagnosticStage::ExitRequested); }
                if force_exit.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let engine = app.state::<Arc<Engine>>().inner().clone();
                // Keep the process alive until workers finish saving receipts and errors.
                // Child exit alone is insufficient: SQLite writes follow process output collection.
                if closing.load(std::sync::atomic::Ordering::SeqCst)
                    && engine.canvas_oauth_stopped()
                    && engine.await_worker_shutdown(0)
                    && director_core::providers::await_local_shutdown(0)
                    && director_core::text_executor::await_local_shutdown(0)
                    && director_core::media_probe::await_local_shutdown(0)
                {
                    return;
                }
                api.prevent_exit();
                if closing.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                if let Some(v) = &run_verification { v.diagnostics.mark(DiagnosticStage::ShutdownStarted); }
                engine.stop.store(true, std::sync::atomic::Ordering::SeqCst);
                engine.cancel_canvas_oauth();
                for window in app.webview_windows().values() {
                    let _ = window.hide();
                }
                let handle = app.clone();
                let deadline_handle = handle.clone();
                let force_exit = force_exit.clone();
                let shutdown_verification = run_verification.clone();
                let deadline_verification = run_verification.clone();
                // Independent of potentially blocked OS/helper/file I/O below.
                // Normal completion exits first and terminates this watchdog.
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    force_exit.store(true, std::sync::atomic::Ordering::SeqCst);
                    if let Some(v) = &deadline_verification { v.diagnostics.mark(DiagnosticStage::NativeExitIssued); }
                    deadline_handle.exit(code.unwrap_or(0));
                });
                std::thread::spawn(move || {
                    // This includes the native shutdown helpers and final receipt
                    // writes. An OS/file-I/O fault must not leave a hidden desktop
                    // process waiting forever; durable submit intent is recovered
                    // as unknown, and durable cloud IDs are queried after restart.
                    director_core::providers::shutdown();
                    director_core::text_executor::shutdown();
                    director_core::text_cli::shutdown();
                    director_core::media_probe::shutdown();
                    loop {
                        let providers_done = director_core::providers::await_local_shutdown(1);
                        let text_done = director_core::text_executor::await_local_shutdown(1);
                        let media_done = director_core::media_probe::await_local_shutdown(1);
                        let workers_done = engine.await_worker_shutdown(1);
                        if providers_done && text_done && media_done && workers_done && engine.canvas_oauth_stopped() {
                            if let Some(v) = &shutdown_verification {
                                v.diagnostics.mark(DiagnosticStage::ShutdownFinished);
                                v.diagnostics.mark(DiagnosticStage::NativeExitIssued);
                            }
                            handle.exit(code.unwrap_or(0));
                            break;
                        }
                    }
                });
            }
        });
}
