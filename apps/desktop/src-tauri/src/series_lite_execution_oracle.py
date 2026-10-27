"""Development-only original-tool oracle; synthetic temporary JOBs, no model/media."""
import sys, json, tempfile, hashlib, importlib.util, shutil, subprocess, copy
from pathlib import Path
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[4]
BASE = REPO / 'rules/series-lite/20261027-v0.1.2'
sys.path.insert(0, str(BASE / 'scripts'))
import episode_execution as execution
from context_packet import canonical

spec = importlib.util.spec_from_file_location('original_tests', BASE / 'tests/test_episode_execution.py')
tests = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tests)
fixture = tests.EpisodeExecutionTests()
fixture.setUp()
def files(root):
    return {p.relative_to(root).as_posix(): p.read_bytes().decode('utf-8')
            for p in sorted(root.rglob('*')) if p.is_file() and p.name != '.state.lock'}
base_files = files(fixture.job)
out = {'sourceCommit': '6dea8d6c57dde38f04a4d71f05f4c039004dd4a9',
       'episodeExecutionSha256': hashlib.sha256((BASE / 'scripts/episode_execution.py').read_bytes()).hexdigest(),
       'files': base_files, 'cases': []}

def write(root, values):
    for path, body in values.items():
        p = root / path; p.parent.mkdir(parents=True, exist_ok=True); p.write_bytes(body.encode('utf-8'))

scenarios = ['fresh', 'after_sync_write', 'before_export', 'after_preview_write', 'after_export_write',
             'unrelated_registry', 'related_registry', 'usage_annotation', 'plan_drift', 'preview_drift',
             'baseline_drift', 'source_mismatch', 'missing_registry_asset', 'corrupt_head', 'output_overlaps_input',
             'journal_output_drift', 'inputs_during_execution', 'season_valid', 'season_after_drift','numeric_episode_request']
try:
    for scenario in scenarios:
        with tempfile.TemporaryDirectory(prefix='lite-execution-oracle-') as temporary:
            root = Path(temporary).resolve(); write(root, base_files)
            fixture.job = root
            case = {'name': scenario}
            if scenario=='numeric_episode_request':
                p=root/'execution-request.json';p.write_bytes(p.read_bytes().replace(b'"episode": 1,',b'"episode": 1.0,'))
            if scenario.startswith('season_'):
                source_hash=fixture.read('bundle.json')['sourceHash']
                scope={'version':'series-season-scope-v1','id':'S1','revision':1,'seasonNumber':1,'status':'locked','sourceHash':source_hash,
                  'readingScope':[{'number':1,'sceneIds':['1-1','1-2']}],'episodes':[{'number':1,'sceneIds':['1-1','1-2']}],
                  'target':{'minSeconds':7200,'maxSeconds':9000,'planningSeconds':8100},
                  'ending':{'episode':1,'sceneId':'1-2','sourceRef':'E01:R4','closure':'闭门完成。','hook':'下一任务。','estimate':{'minSeconds':7200,'maxSeconds':8600}},
                  'confirmation':{'kind':'delegated','basis':'合成季范围确认'},
                  'durationException':{'basis':'合成短样本，不是真实季长','confirmation':{'kind':'delegated','basis':'合成授权'}}}
                scope_hash=hashlib.sha256(canonical(scope)).hexdigest()
                fixture.save('season.json',scope)
                b=fixture.read('bundle.json');b.update(productionScope=scope,scopeHash=scope_hash,seasonDelivery='preview');fixture.save('bundle.json',b)
                ix=fixture.read('source-index.json');ix.update(productionScope=scope,scopeHash=scope_hash,readingScope=scope['readingScope']);fixture.save('source-index.json',ix)
                state=fixture.read('state.json');state.update(productionScope=scope,scopeHash=scope_hash,sourceIndexHash=hashlib.sha256((root/'source-index.json').read_bytes()).hexdigest());fixture.save('state.json',state)
                registry=fixture.read('registry/v000001.json');registry.update(productionScope=scope,productionScopeHash=scope_hash)
                for a in registry['assets']:a['scopeUses']=[{'episode':1,'sceneId':'1-1'},{'episode':1,'sceneId':'1-2'}]
                fixture.save('registry/v000001.json',registry)
                head=fixture.read('registry/current.json');head['sha256']=hashlib.sha256((root/'registry/v000001.json').read_bytes()).hexdigest();fixture.save('registry/current.json',head)
                fixture.build_packet(output='packet-season.json')
                request=fixture.read('execution-request.json');request.update(packet='packet-season.json',seasonScope='season.json');fixture.save('execution-request.json',request)
            node = execution.node
            def intercepted(script, args):
                is_export = script == 'storyboard.mjs' and args[0] == 'export'
                if scenario in ('before_export', 'journal_output_drift') and is_export:
                    raise RuntimeError('synthetic interruption before export')
                if scenario == 'after_preview_write' and is_export:
                    preview = Path(args[args.index('--output') + 1]); preview.parent.mkdir(parents=True, exist_ok=True)
                    preview.write_text('partial bytes retained for inspection', encoding='utf-8')
                    raise RuntimeError('synthetic partial dual write')
                result = node(script, args)
                if scenario == 'after_sync_write' and script == 'assets.mjs' and args[0] == 'sync':
                    raise RuntimeError('synthetic interruption after sync')
                if scenario == 'after_export_write' and is_export:
                    raise RuntimeError('synthetic interruption after export')
                if scenario == 'inputs_during_execution' and script == 'storyboard.mjs' and args[0] == 'verify':
                    with (root / 'direction.txt').open('a', encoding='utf-8') as f: f.write('drift')
                return result
            if scenario == 'baseline_drift':
                b=fixture.read('bundle.json');b['episodes'][0]['settings']['visualStyle']='rewritten';fixture.save('bundle.json',b)
            if scenario == 'source_mismatch':
                b=fixture.read('bundle.json');b['episodes'][0]['source']['sourceRows'][0]['text']='changed';fixture.save('bundle.json',b)
            if scenario == 'missing_registry_asset':
                b=fixture.read('bundle.json');b['assets'][0]['assetId']='unknown';fixture.save('bundle.json',b)
            if scenario == 'corrupt_head':
                h=fixture.read('registry/current.json');h['sha256']='a'*64;fixture.save('registry/current.json',h)
            if scenario == 'output_overlaps_input':
                fixture.save('adoption/protected.txt','protected')
            case['beforePatches']={k:v for k,v in files(root).items() if base_files.get(k)!=v}
            try:
                request,signature,fingerprint,snapshot=execution.inputs(root,'execution-request.json')
                case['inputs']={'signature':json.loads(canonical(signature)), 'fingerprint':fingerprint, 'snapshot':snapshot}
            except Exception as e:
                case['inputsError']=str(e)
            try:
                with patch.object(execution,'node',side_effect=intercepted):
                    first=execution.run(root,'execution-request.json','adoption')
                case['first']=first
            except Exception as e:
                case['firstError']=str(e)
            journal = root/'adoption/execution.json'
            if journal.exists(): case['initialSteps']=list(json.loads(journal.read_text(encoding='utf-8'))['steps'])
            case['receiptInitiallyExists']=(root/'adoption/adoption.json').exists()
            before_mutation=files(root)
            if scenario in ('unrelated_registry','related_registry'): fixture.asset_change(revise=scenario=='related_registry')
            if scenario=='usage_annotation': fixture.record_later_uses()
            if scenario=='plan_drift':
                with (root/'plan.json').open('a',encoding='utf-8') as f:f.write(' ')
            if scenario=='preview_drift':
                with (root/'adoption/export-0/preview.md').open('a',encoding='utf-8') as f:f.write('\nchanged\n')
            if scenario=='journal_output_drift':
                with (root/'adoption/check.json').open('a',encoding='utf-8') as f:f.write(' ')
            if scenario=='season_after_drift':
                with (root/'season.json').open('a',encoding='utf-8') as f:f.write(' ')
            case['postPatches']={k:v for k,v in files(root).items() if before_mutation.get(k)!=v}
            try: case['resumed']=execution.run(root,'execution-request.json','adoption')
            except Exception as e: case['resumeError']=str(e)
            receipt=root/'adoption/adoption.json'
            if receipt.exists():
                saved=json.loads(receipt.read_text(encoding='utf-8'))
                try: execution.verify_adoption(root,'adoption/adoption.json');case['verifyOk']=True
                except Exception as e:case['verifyOk']=False;case['verifyError']=str(e)
                if case.get('verifyOk'):
                    case['markdown']=(root/saved['preview']).read_text(encoding='utf-8')
                    case['catalog']=(root/(saved['preview']+'.assets.json')).read_text(encoding='utf-8')
                    case['fieldCapacity']=json.loads((root/'adoption/verify.json').read_text(encoding='utf-8'))['fieldCapacity']
                    case['verifyReport']=json.loads((root/'adoption/verify.json').read_text(encoding='utf-8'))
                    case['verifyReport'].pop('assetsIndex',None)
                    case['contentReview']=saved['contentReview']
            out['cases'].append(case)
finally:
    fixture.doCleanups()
view_registry={'version':'series-assets-v1','id':'R','revision':1,'sourceHash':'a'*64,
 'assets':[{'id':f'A{i:04d}','revision':1,'design':{'text':f'设计{i}','dependencies':([{'assetId':f'A{i+1:04d}'}] if i<11 else [])}} for i in range(1,13)],
 'requests':[{'id':'this','uses':[{'episode':1}]},{'id':'other','uses':[{'episode':2}]}]}
view_bundle={'assets':[{'key':'人物CH','assetId':'A0001'}],'episodes':[{'number':1,'draft':{'units':[{'bindings':['人物CH']}]}}]}
out['assetViews']=[]
for name in ['eleven_levels','unrelated_changed','transitive_changed','later_usage_annotation','other_request_changed','own_request_changed','missing_parent','cycle_view_only']:
    r=copy.deepcopy(view_registry)
    if name=='unrelated_changed':r['assets'][-1]['design']['text']='其它设计'
    if name=='transitive_changed':r['assets'][9]['design']['text']='真正依赖变更'
    if name=='later_usage_annotation':r['usageAnnotations']=[{'episode':2,'assetId':'A0001'}]
    if name=='other_request_changed':r['requests'][1]['reason']='其它集请求'
    if name=='own_request_changed':r['requests'][0]['reason']='本集请求'
    if name=='missing_parent':r['assets'][9]['design']['dependencies']=[{'assetId':'unknown'}]
    if name=='cycle_view_only':r['assets'][9]['design']['dependencies']=[{'assetId':'A0001'}]
    case={'name':name,'registry':r,'bundle':view_bundle}
    with tempfile.TemporaryDirectory(prefix='lite-asset-view-') as tmp:
        root=Path(tmp).resolve();raw=canonical(r)+b'\n';(root/'registry').mkdir();(root/'registry/v000001.json').write_bytes(raw)
        (root/'registry/current.json').write_bytes(canonical({'version':'series-assets-head-v1','id':'R','revision':1,'sha256':hashlib.sha256(raw).hexdigest()}))
        try:case['fingerprint'],_=execution.asset_view(root,'registry',view_bundle)
        except Exception as e:case['error']=str(e)
    out['assetViews'].append(case)
out['fieldBudgets']=[]
for raw in ['无','待确认','钥匙、包','钥匙\\、扣、包\\\\绳','\ufeff钥匙\ufeff、包','盒'*90,'😀'*90]:
    md='\ufeff**【标准化剧本】**\r\n第十集\r\n10-1\r\n场景：屋\r\n人物：甲\r\n道具：'+raw+'\r\n'
    with tempfile.TemporaryDirectory(prefix='lite-budget-') as tmp:
        path=Path(tmp)/'sample.md';path.write_bytes(md.encode('utf-8'))
        result=subprocess.run(['node',str(BASE/'scripts/lingmo-field-budget.mjs'),str(path)],capture_output=True,check=True)
        out['fieldBudgets'].append({'markdown':md,'report':json.loads(result.stdout)})
target=REPO/'apps/desktop/src-tauri/src/series_lite_execution_golden.json'
target.write_text(json.dumps(out,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(json.dumps({'cases':len(out['cases']),'validResumes':sum('resumed' in c for c in out['cases']),'golden':str(target)},ensure_ascii=False))
