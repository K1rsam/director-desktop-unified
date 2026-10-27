"""Development-only oracle; production Rust never invokes Python.

Run from any cwd. Only synthetic fixtures are read/written; frozen source is
imported without modification. Record complete output bytes, not self-authored
expected Rust hashes. Workspace integration is separately fail-closed.
"""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile

HERE = Path(__file__).resolve().parent
BASE = HERE.parents[3] / 'rules/series-lite/20261027-v0.1.2'
sys.path.insert(0, str(BASE / 'scripts'))
import job_packet as jp


def fixtures(role='author'):
    files = {
        'source.txt': '第1集\r\n甲：“不要改字，1.250000001秒。”\r\n末尾空格  \n',
        'direction.txt': '有效方向；test_fixture 不是真实审核。',
        'facts.txt': '当下事实\n保留全部必要事实',
        'assets.txt': '完整采用设计\n人物、场景、道具、空间及声音设计。',
        'entry.txt': 'PRELIMINARY，未获前集实际尾稿',
        'review.md': 'test_fixture: supplied dependency review evidence; no quality claim.',
        'baseline.json': (BASE / 'tests/fixtures/visual-baseline.json').read_text(encoding='utf-8'),
    }
    request = {'version': jp.REQUEST_VERSION, 'episode': 1, 'role': role,
               'review': {'path': 'review.md'},
               'inputs': [{'kind': k, 'path': p} for k, p in [
                   ('source', 'source.txt'), ('direction', 'direction.txt'),
                   ('state', 'facts.txt'), ('assets', 'assets.txt'), ('entry', 'entry.txt')]]}
    request['inputs'].append({'kind': 'visual-baseline', 'path': 'baseline.json', 'pointer': ''})
    if role != 'author':
        files['work.json'] = '{"draft":{"text":"全部当前稿","seconds":1.250000001123456789123456789},"plan":{"beats":["完整统筹"]},"selection":{"picked":["声音","幻想"]}}'
        request['inputs'] += [{'kind': k, 'path': 'work.json', 'pointer': '/' + k} for k in ['draft', 'plan', 'selection']]
    return files, request


def main():
    cases = []
    def add(name, mutate=None, role='author', budget=jp.DEFAULT_MAX_BYTES):
        files, request = fixtures(role)
        if mutate:
            mutate(files, request)
        manifest = files.pop('!manifest', None) or json.dumps(request, ensure_ascii=False)
        files['request.json'] = manifest
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for path, value in files.items():
                (root / path).parent.mkdir(parents=True, exist_ok=True)
                (root / path).write_bytes(value.encode('utf-8'))
            case = {'name': name, 'files': files, 'maxBytes': budget}
            try:
                packet, raw = jp.assemble(root, 'request.json', budget, BASE)
                case.update(ok=True, encoded=raw.decode('utf-8'), sha256=jp.sha256(raw),
                            inputFingerprint=packet['inputFingerprint'])
            except (ValueError, OSError, RuntimeError) as error:
                case.update(ok=False, error=str(error))
            cases.append(case)

    add('author_complete')
    add('finisher_complete', role='finisher')
    add('reviewer_complete', role='reviewer')
    add('first_use_rules_order', lambda f, r: r.update(methods=['fantasy-expression-methods', 'acting-blocking-methods'], contracts=['system-presentation', 'shot-handoffs', 'asset-registry-contract']))
    def decimals(f, r):
        f['numbers.json'] = '{"a/b":{"~key":[{"":1.250000001123456789123456789},null]},"one":{"v":[1e309,-0.0000000,1.0,123456789012345678901234567890,1.200e-7]},"two":{"v":[1e309,-0.0000000,1.0,123456789012345678901234567890,1.200e-7]}}'
        r['inputs'] += [{'kind': 'plan', 'path': 'numbers.json', 'pointer': '/a~1b/~0key/0/'}, {'kind': 'selection', 'path': 'numbers.json', 'pointer': '/one'}, {'kind': 'issues', 'path': 'numbers.json', 'pointer': '/two'}]
    add('decimals_pointer_and_duplicate_projection', decimals)
    def dedup(f, r):
        f['entry-copy.txt'] = f['source.txt']
        r['inputs'] += [{'kind': 'entry', 'path': 'entry-copy.txt'}, {'kind': 'state', 'path': 'source.txt'}]
    add('same_text_distinct_origins', dedup)
    add('whole_source_300_lines', lambda f, r: f.update({'source.txt': f['source.txt'] * 300}))
    add('unicode_json_key_order_controls', lambda f, r: (f.update({'unicode.json': '{"𐀀":"\u001f\b\f\t","\ue000":"中文\u2028","😀":"é"}'.replace('\u001f', '\\u001f').replace('\b', '\\b').replace('\f', '\\f').replace('\t', '\\t')}), r['inputs'].append({'kind': 'issues', 'path': 'unicode.json', 'pointer': ''})))
    add('zero_episode', lambda f, r: r.update(episode=0))
    add('negative_episode', lambda f, r: r.update(episode=-7))
    add('large_integer_episode', lambda f, r: r.update(episode=123456789012345678901234567890123456789012345678901))
    add('text_baseline', lambda f, r: r['inputs'][-1].pop('pointer'))
    add('json_review', lambda f, r: (f.update({'review.json': '{"read":["complete source","necessary adopted assets"]}'}), r.update(review={'path': 'review.json', 'pointer': ''})))
    for kind in ['source', 'direction', 'state', 'assets', 'entry', 'visual-baseline']:
        add('missing_' + kind, lambda f, r, kind=kind: r.update(inputs=[v for v in r['inputs'] if v['kind'] != kind]))
    for role in ['finisher', 'reviewer']:
        for kind in ['draft', 'plan', 'selection']:
            add(role + '_missing_' + kind, lambda f, r, kind=kind: r.update(inputs=[v for v in r['inputs'] if v['kind'] != kind]), role=role)
    add('baseline_candidate', lambda f, r: f.update({'baseline.json': json.dumps({**json.loads(f['baseline.json']), 'status': 'candidate'}, ensure_ascii=False)}))
    add('baseline_duplicate', lambda f, r: r['inputs'].append(copy.deepcopy(r['inputs'][-1])))
    add('empty_review', lambda f, r: f.update({'review.md': ' \n '}))
    add('review_bool', lambda f, r: (f.update({'review.md': 'true'}), r.update(review={'path': 'review.md', 'pointer': ''})))
    add('budget_rejects', budget=100)
    add('duplicate_method', lambda f, r: r.update(methods=['director-refinement', 'director-refinement']))
    add('unknown_contract', lambda f, r: r.update(contracts=['invented']))
    add('episode_bool', lambda f, r: r.update(episode=True))
    add('episode_decimal', lambda f, r: r.update(episode=1.0))
    add('unknown_input_kind', lambda f, r: r['inputs'][0].update(kind='transcript'))
    add('duplicate_manifest_key', lambda f, r: f.update({'!manifest': '{"version":1,"version":2}'}))
    for name, raw, pointer in [
        ('duplicate_key', '{"secret":1,"secret":2}', ''),
        ('nonfinite', '{"secret":NaN}', ''),
        ('unpaired_surrogate', '{"secret":"\\ud800"}', ''),
        ('bad_pointer', '{"items":[1]}', '/items/01'),
        ('escaped_pointer', '{}', '/~2'),
        ('large_index', '[1]', '/99999999999999999999999999999999999'),
        ('missing_key', '{}', '/secret'),
        ('uri_pointer', '{}', '#/secret'),
    ]:
        add(name, lambda f, r, raw=raw, pointer=pointer: (f.update({'projection.json': raw}), r['inputs'].append({'kind': 'issues', 'path': 'projection.json', 'pointer': pointer})))
    out = {
        'upstreamCommit': '6dea8d6c57dde38f04a4d71f05f4c039004dd4a9',
        'sourceHashes': {name: hashlib.sha256((BASE / 'scripts' / name).read_bytes()).hexdigest() for name in ['job_packet.py', 'context_packet.py', 'visual_baseline.py']},
        'scope': 'Synthetic standalone packets; workspace adapter is separately required.',
        'cases': cases,
    }
    target = HERE / 'series_lite_job_packet_golden.json'
    target.write_text(json.dumps(out, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'cases': len(cases), 'pass': sum(c['ok'] for c in cases), 'bytes': target.stat().st_size}))


if __name__ == '__main__':
    main()
