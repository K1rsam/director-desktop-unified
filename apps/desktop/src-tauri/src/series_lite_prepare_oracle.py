"""Development-only frozen 0.4.1 one-call preparation/reading oracle.

Only synthetic fixtures; the normal desktop runtime never invokes Python/Node.
"""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile

HERE = Path(__file__).resolve().parent
BASE = HERE.parents[3] / 'rules/series-lite/20261028-v0.4.1'
sys.path.insert(0, str(BASE / 'scripts'))
import prepare_episode as pe
import context_packet as cp


def fixtures():
    base = json.loads((BASE / 'tests/fixtures/lingmo-v7-bundle.json').read_text(encoding='utf-8'))
    base.update(testFixture=False, sourceHash='a' * 64, sourceLockHash='b' * 64,
                generationTextVersion='voice-delivery-v1', generationBoundaryVersion='video-prompt-v1',
                assetRegistry={'id': 'synthetic-registry', 'revision': 1})
    ep = base['episodes'][0]
    visual = json.loads((BASE / 'tests/fixtures/visual-baseline.json').read_text(encoding='utf-8'))
    ep['settings'].update(visualBaseline=visual, visualStyle=visual['visualStyle'], fantasyMode='off')
    assets = []
    for n, asset in enumerate(base['assets'], 1):
        row = copy.deepcopy(asset)
        row.update(id='A%04d' % n, revision=1,
                   design={'state': 'TEXT_ADOPTED', 'text': '完整采用设计',
                           'full': {'body': '完整分镜必要设计与未知约束，不能删掉抓握袖口。' * 20},
                           'unknownConstraint': '遮口鼻，不可露嘴', 'dependencies': []})
        assets.append(row)
        asset.update(assetId=row['id'], assetRevision=1)
    registry = {'version': 'series-assets-v1', 'id': 'synthetic-registry', 'revision': 1,
                'sourceHash': base['sourceHash'], 'assets': assets,
                'imageDesigns': {'unused': '制图记录不供给分镜'},
                'requests': [{'history': '无关请求历史不供给分镜'}]}
    return base, registry


def main():
    cases = []
    def add(name, mutation=None, role='author', options=None):
        base, registry = fixtures()
        files = {}
        if role == 'author':
            for key in ('plan', 'selection', 'draft'):
                base['episodes'][0][key] = {}
        if mutation:
            mutation(base, registry, files)
        snapshot = cp.canonical(registry)
        files.update({'base.json': cp.canonical(base).decode('utf-8'),
                      'registry/v000001.json': snapshot.decode('utf-8'),
                      'registry/current.json': cp.canonical({'version': 'series-assets-head-v1',
                          'id': registry['id'], 'revision': registry['revision'],
                          'sha256': cp.sha256(snapshot)}).decode('utf-8')})
        options = options or {}
        case = {'name': name, 'files': files, 'role': role, 'options': options}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for path, content in files.items():
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(content.encode('utf-8'))
            try:
                result = pe.prepare(root, 'base.json', 'registry', role, 'prepared', **options)
                case.update(ok=True, result=result,
                            outputs={p.relative_to(root).as_posix(): p.read_bytes().decode('utf-8')
                                     for p in (root / 'prepared').rglob('*') if p.is_file()})
            except (ValueError, OSError, RuntimeError, KeyError, TypeError) as error:
                case.update(ok=False, error=str(error), outputExists=(root / 'prepared').exists())
        cases.append(case)

    add('direct_author')
    add('direct_finisher', role='finisher')
    add('direct_reviewer', role='reviewer')
    add('effective_context_pointer', lambda b,r,f: f.update({'context.json': '{"effective":{"entry":"右手仍握钥匙","constraint":"面罩遮口鼻，不露嘴"},"history":"旧核查过程不供本岗"}'}),
        options={'context':'context.json','context_pointer':'/effective'})
    add('effective_direction_and_issues', lambda b,r,f: f.update({'extra.json':'{"direction":{"lighting":"冷光"},"issues":[{"coordinate":"U1","detail":"保持袖口"}]}'}),
        options={'direction':'extra.json','direction_pointer':'/direction','issues':'extra.json','issues_pointer':'/issues'})
    def chain(b,r,f,length=3):
        child = r['assets'][0]
        for n in range(length-1):
            parent = copy.deepcopy(child)
            parent.update(id='A9%03d' % n, key='母版%dCH' % n, name='母版%d' % n)
            parent['design']['dependencies'] = []
            child['design']['dependencies'] = [{'assetId': parent['id'], 'assetRevision': 1, 'purpose': '真实设计继承'}]
            r['assets'].append(parent)
            child = parent
    add('three_layer_exact_design_dependencies',chain)
    add('eleven_layer_exact_design_dependencies',lambda b,r,f:chain(b,r,f,11))
    def cycle(b,r,f):
        chain(b,r,f)
        r['assets'][-1]['design']['dependencies']=[{'assetId':r['assets'][0]['id'],'assetRevision':1,'purpose':'合成环'}]
    add('cyclic_design_dependency',cycle)
    add('wrong_design_revision',lambda b,r,f:b['assets'][0].update(assetRevision=2))
    add('unadopted_design',lambda b,r,f:r['assets'][0]['design'].update(state='UNADOPTED'))
    add('missing_bound_asset',lambda b,r,f:r['assets'].pop(0))
    add('missing_context_pointer',options={'context':'missing.json','context_pointer':'/effective'})
    add('missing_non_author_draft',lambda b,r,f:b['episodes'][0].update(draft={}),role='finisher')
    add('too_small_packet_budget',options={'max_bytes':100})
    add('method_and_contract',options={'methods':['fantasy-expression-methods'],'contracts':['system-presentation']})
    add('source_crlf_and_code_fence',lambda b,r,f:b['episodes'][0]['source']['sourceRows'][0].update(text='原句。\r\n```原样资料```\r\n'))
    target = HERE / 'series_lite_prepare_golden.json'
    target.write_text(json.dumps({'upstreamCommit':'f855227c8406b38093d03c777fe24a0b0b4f87dc',
        'sourceHashes':{name:hashlib.sha256((BASE/'scripts'/name).read_bytes()).hexdigest()
                        for name in ('prepare_episode.py','packet_view.py','job_packet.py')},
        'cases':cases},ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({'cases':len(cases),'pass':sum(c['ok'] for c in cases),'bytes':target.stat().st_size}))


if __name__ == '__main__':
    main()
