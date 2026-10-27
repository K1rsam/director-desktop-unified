"""Python consumers share the Node season policy; no duplicate boundary engine."""
import json
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile


def verify_previous(root, scope, previous_path=None, expected_hash=None):
    if scope is None or scope['seasonNumber'] == 1:
        if previous_path is not None:
            raise ValueError('仅续季需要 --previous-handoff，不能将其套入首季/旧全剧')
        return None
    if previous_path is None:
        raise ValueError('续季派发必须提供 --previous-handoff 上季实际closed包；计划或自填hash不构成闭季证据')
    root = Path(root).resolve()
    path = Path(previous_path)
    path = (root / path).resolve()
    try:
        relative_path = path.relative_to(root).as_posix()
    except ValueError:
        raise ValueError('请先把完整上季对接目录复制到本JOB内，再提供previous-handoff路径') from None
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if expected_hash is not None and expected_hash != digest:
        raise ValueError('上季闭季包发生变化，不能沿用旧续季派发状态')
    with tempfile.TemporaryDirectory(prefix='series-continuation-') as folder:
        next_scope = Path(folder) / 'scope.json'
        next_scope.write_text(json.dumps(scope, ensure_ascii=False), encoding='utf-8')
        command = [os.environ.get('SERIES_NODE', 'node'),
                   str(Path(__file__).with_name('season-handoff.mjs')), 'verify-next',
                   '--input', str(path), '--root', str(path.parent), '--next-scope', str(next_scope)]
        try:
            result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8')
        except OSError as error:
            raise ValueError('续季闭季校验需要Node：' + str(error)) from None
        if result.returncode:
            raise ValueError('上季闭季/续季游标拒绝派发：' + result.stderr.strip())
        report = json.loads(result.stdout)
    return {'path': relative_path, 'sha256': digest,
            'previousScopeHash': report['previousScopeHash'], 'nextEpisode': report['nextEpisode']}


def project_index(index_path, scope_path=None):
    index_path = Path(index_path).resolve()
    index = json.loads(index_path.read_text(encoding='utf-8'))
    embedded = index.get('productionScope')
    if embedded is None and scope_path is None:
        if any(key in index for key in ('scopeHash', 'readingScope')):
            raise ValueError('季范围凭据不完整，不能降级为全剧派发')
        return index
    with tempfile.TemporaryDirectory(prefix='series-season-') as folder:
        if scope_path is None:
            scope_path = Path(folder) / 'scope.json'
            scope_path.write_text(json.dumps(embedded, ensure_ascii=False), encoding='utf-8')
        command = [os.environ.get('SERIES_NODE', 'node'),
                   str(Path(__file__).with_name('season-scope.mjs')), 'index',
                   '--input', str(Path(scope_path).resolve()), '--index', str(index_path), '--output', '-']
        try:
            result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8')
        except OSError as error:
            raise ValueError('选季派发需要 Node 运行季范围校验器：' + str(error)) from None
        if result.returncode:
            raise ValueError('季范围拒绝派发：' + result.stderr.strip())
        return json.loads(result.stdout)


def check_state_scope(state, index):
    scope = index.get('productionScope')
    if scope is None:
        if 'productionScope' in state or 'scopeHash' in state:
            raise ValueError('季范围与冻结索引不一致')
        return
    if state.get('productionScope') != scope or state.get('scopeHash') != index.get('scopeHash'):
        raise ValueError('季范围状态与冻结索引不一致')
    selected = [episode['number'] for episode in scope['episodes']]
    active = state.get('scope', [])
    if not active or active != [number for number in selected if number in active]:
        raise ValueError('派发范围超出已锁定季范围或顺序漂移')
    groups = [number for group in state.get('groups', []) for number in group['episodes']]
    if groups != active or set(state.get('episodes', {})) != {str(number) for number in active}:
        raise ValueError('任务队列与已锁定季范围不一致')
