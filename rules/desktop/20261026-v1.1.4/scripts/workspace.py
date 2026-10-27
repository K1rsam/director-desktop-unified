#!/usr/bin/env python3
"""Portable file evidence and serial-within-group scheduling. No model calls."""
import argparse
from contextlib import contextmanager
import hashlib
from io import BytesIO
import json
import os
from pathlib import Path
import sys
import tempfile
from xml.etree import ElementTree as ET
import zipfile

STAGES = ('intake', 'assets', 'direction', 'delivery')
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
R = '{http://schemas.openxmlformats.org/officeDocument/2006/relationships}'
M = '{http://schemas.openxmlformats.org/officeDocument/2006/math}'
MC = '{http://schemas.openxmlformats.org/markup-compatibility/2006}'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read_json(path):
    return json.loads(Path(path).read_text(encoding='utf-8'))


def write_json(path, data):
    # ponytail: one short workspace lock; no scheduler daemon or database.
    fd, temporary = tempfile.mkstemp(prefix='.state-', dir=str(path.parent))
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as out:
            json.dump(data, out, ensure_ascii=False, indent=2)
            out.write('\n')
            out.flush()
            os.fsync(out.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


@contextmanager
def locked(root):
    require(root.is_dir(), '工作区不存在；先 extract')
    with (root / '.state.lock').open('a+b') as handle:
        if os.name == 'nt':
            import msvcrt
            handle.seek(0)
            handle.write(b'0')
            handle.flush()
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_LOCK, 1)
        else:
            import fcntl
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
        try:
            yield
        finally:
            if os.name == 'nt':
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(handle.fileno(), fcntl.LOCK_UN)


def local_path(root, value):
    require(isinstance(value, str) and bool(value), '成果路径须为非空字符串')
    path = (root / value).resolve()
    require(path.is_relative_to(root), '成果路径不能逃逸工作区: ' + value)
    return path


def file_record(root, value):
    path = local_path(root, str(value))
    require(path.is_file() and path.stat().st_size > 0, '证据/成果文件缺失或为空: ' + str(value))
    return {'path': path.relative_to(root).as_posix(), 'sha256': sha(path)}


def valid_record(root, record):
    try:
        return file_record(root, record['path']) == record
    except (OSError, ValueError, KeyError):
        return False


def extract(source, root):
    suffix = source.suffix.lower()
    require(suffix in ('.txt', '.md', '.docx'), '只支持 UTF-8 txt/md 或 docx')
    raw = source.read_bytes()
    excluded = []
    excluded_details = []
    if suffix == '.docx':
        with zipfile.ZipFile(BytesIO(raw)) as package:
            body = ET.fromstring(package.read('word/document.xml')).find(W + 'body')
            require(body is not None, 'DOCX 缺主正文')
            rels = {}
            if 'word/_rels/document.xml.rels' in package.namelist():
                rels = {r.get('Id'): r.attrib for r in ET.fromstring(package.read('word/_rels/document.xml.rels'))}
            styles = {}
            if 'word/styles.xml' in package.namelist():
                styles = {style.get(W + 'styleId'): (style, i + 1) for i, style in enumerate(ET.fromstring(package.read('word/styles.xml')).findall(W + 'style'))}
            default_style = next((key for key, (style, _) in styles.items() if style.get(W + 'type') == 'paragraph'
                                  and style.get(W + 'default') in ('1', 'true', 'on')), None)
            paragraphs = []
            omitted = {W + 'txbxContent': 'textboxes', W + 'drawing': 'drawings', W + 'pict': 'drawings',
                       W + 'object': 'objects', W + 'sym': 'symbol', W + 'altChunk': 'altChunk',
                       W + 'subDoc': 'subDocument', W + 'numPr': 'numbering',
                       M + 'oMath': 'math', M + 'oMathPara': 'math', MC + 'AlternateContent': 'alternateContent'}

            def children(node, path):
                counts = {}
                for child in node:
                    counts[child.tag] = counts.get(child.tag, 0) + 1
                    name = child.tag
                    for namespace, prefix in ((W, 'w'), (M, 'm'), (MC, 'mc')):
                        if name.startswith(namespace):
                            name = prefix + ':' + name[len(namespace):]
                            break
                    yield child, '%s/%s[%s]' % (path, name, counts[child.tag])

            def skip(node, path):
                if node.tag in (W + 'del', W + 'moveFrom'):
                    return True
                kind = omitted.get(node.tag)
                if kind is None:
                    return False
                detail = {'kind': kind, 'part': 'word/document.xml', 'path': path}
                if kind == 'symbol':
                    detail.update(font=node.get(W + 'font'), char=node.get(W + 'char'))
                elif kind in ('altChunk', 'subDocument'):
                    rid = node.get(R + 'id')
                    detail.update(relationshipId=rid, target=rels.get(rid, {}).get('Target'))
                elif kind == 'numbering':
                    detail.update({tag: node.find(W + tag).get(W + 'val') for tag in ('numId', 'ilvl') if node.find(W + tag) is not None})
                excluded_details.append(detail)
                return True

            def text_inside(node, path):
                if skip(node, path):
                    return ''
                if node.tag == W + 't':
                    return node.text or ''
                if node.tag == W + 'tab':
                    return '\t'
                if node.tag == W + 'noBreakHyphen':
                    return '\u2011'
                if node.tag == W + 'softHyphen':
                    return '\u00ad'
                if node.tag in (W + 'br', W + 'cr'):
                    return '\n'
                return ''.join(text_inside(child, child_path) for child, child_path in children(node, path))

            def style_numbering(node, path):
                reference = node.find(W + 'pPr/' + W + 'pStyle')
                selected = reference.get(W + 'val') if reference is not None else default_style
                current, seen = selected, set()
                while current in styles and current not in seen:
                    seen.add(current)
                    style, position = styles[current]
                    numbering = style.find(W + 'pPr/' + W + 'numPr')
                    if numbering is not None:
                        detail = {'kind': 'numbering', 'part': 'word/document.xml', 'path': path,
                                  'styleId': selected, 'inheritedFromStyle': current, 'stylePart': 'word/styles.xml',
                                  'stylePath': '/w:styles/w:style[%s]/w:pPr[1]/w:numPr[1]' % position}
                        detail.update({tag: numbering.find(W + tag).get(W + 'val') for tag in ('numId', 'ilvl') if numbering.find(W + tag) is not None})
                        excluded_details.append(detail)
                    parent = style.find(W + 'basedOn')
                    current = parent.get(W + 'val') if parent is not None else None

            def visit(node, path):
                if skip(node, path):
                    return
                if node.tag == W + 'p':
                    style_numbering(node, path)
                    paragraphs.append(text_inside(node, path))
                    return
                for child, child_path in children(node, path):
                    visit(child, child_path)

            visit(body, '/w:body')
            content = '\n'.join(paragraphs) + '\n'
        excluded = ['headers', 'footers', 'comments', 'footnotes', 'endnotes',
                    'textboxes', 'drawings', 'deleted revision text']
        excluded.extend(sorted({item['kind'] for item in excluded_details} - set(excluded)))
    else:
        content = raw.decode('utf-8-sig')
    missing = '; '.join(item['path'] + ' (' + item['kind'] + ')' for item in excluded_details)
    require(bool(content.strip()), '源正文为空，未创建工作区' + ('；发现未提取内容: ' + missing if missing else ''))
    root.mkdir(parents=True, exist_ok=False)
    copy = root / ('source-original' + suffix)
    copy.write_bytes(raw)
    (root / 'source-body.txt').write_text(content, encoding='utf-8')
    manifest = {'version': 'series-extraction-v1', 'sourceName': source.name,
                'sourceCopy': copy.name, 'sha256': sha(copy),
                'bodyHash': sha(root / 'source-body.txt'), 'notExtracted': excluded,
                'notExtractedDetails': excluded_details,
                'dialogueParsed': False,
                'revisionPolicy': 'insertions/moveTo included; deletions/moveFrom excluded' if suffix == '.docx' else None}
    write_json(root / 'extraction.json', manifest)
    return manifest


def verify_source(root):
    manifest = read_json(root / 'extraction.json')
    require(sha(local_path(root, manifest['sourceCopy'])) == manifest['sha256'], '源文件副本变化，不能沿用旧状态')
    require(sha(root / 'source-body.txt') == manifest['bodyHash'], '提取正文变化，不能沿用旧状态')
    return manifest


def init(root, index_path, workers, selection, active_slots=4, batch_size=2, coordinator='/root', input_limit=60000, season_scope=None, previous_handoff=None):
    from season_scope import project_index, check_state_scope, verify_previous
    require(not (root / 'state.json').exists(), '已有状态；拒绝重复 init 或覆盖成果')
    manifest = verify_source(root)
    index = project_index(index_path, season_scope)
    continuation = verify_previous(root, index.get('productionScope'), previous_handoff)
    require(index.get('version') == 'series-source-v1', '源索引 version 必须是 series-source-v1')
    require(index.get('sourceHash') == manifest['sha256'], '索引 sourceHash 与源文件不符')
    episodes = index.get('episodes')
    require(isinstance(episodes, list) and bool(episodes), '源索引必须包含完整集场')
    numbers = []
    for ep in episodes:
        n = ep.get('number')
        require(isinstance(n, int) and not isinstance(n, bool) and n not in numbers, '集号须为唯一整数')
        scenes = ep.get('scenes')
        require(isinstance(scenes, list) and bool(scenes), '每集须有场索引')
        ids = [s.get('id') for s in scenes]
        require(all(isinstance(s, str) and s.strip() for s in ids) and len(set(ids)) == len(ids), '场 ID 须为本集唯一非空字符串')
        numbers.append(n)
    require(active_slots >= 2, '主控只调度；至少需要主控与一个子执行槽')
    require(1 <= batch_size <= 3, 'batch-size 须为 1～3；复杂集用1')
    require(bool(coordinator.strip()) and input_limit > 0, '主控身份与上下文阈值不能为空或非正')
    workers = min(4, active_slots - 1) if workers is None else workers
    require(1 <= workers <= 4, 'workers 须为 1～4，表示逻辑队列，不含主控')
    source_order = [ep['number'] for ep in index.get('readingScope', [])] or list(numbers)
    if selection:
        require(len(set(selection)) == len(selection) and set(selection).issubset(numbers), '指定集必须存在且不重复')
        numbers = [n for n in numbers if n in selection]
    groups = []
    count = min(workers, len(numbers))
    start = 0
    for i in range(count):
        size = len(numbers) // count + (i < len(numbers) % count)
        groups.append({'id': i + 1, 'episodes': numbers[start:start + size]})
        start += size
    if 'productionScope' in index:
        write_json(root / 'source-index.json', index)
    else:
        (root / 'source-index.json').write_bytes(index_path.read_bytes())
    state = {'version': 'series-workspace-v1', 'sourceHash': manifest['sha256'],
             'sourceIndexHash': sha(root / 'source-index.json'), 'scope': numbers, 'sourceOrder': source_order,
             'groups': groups, 'episodes': {}, 'history': [],
             'qualityAcceptance': 'not_inferred', 'usage': None, 'cost': None,
             'dispatch': {'version': 'series-dispatch-v1', 'coordinator': coordinator,
                          'activeSlots': active_slots, 'batchSize': batch_size,
                          'inputTokenLimit': input_limit, 'workers': {}}}
    for group in groups:
        for n in group['episodes']:
            state['episodes'][str(n)] = {'status': 'pending', 'group': group['id']}
    if 'productionScope' in index:
        state.update(productionScope=index['productionScope'], scopeHash=index['scopeHash'])
        if continuation is not None:
            state['continuationEvidence'] = continuation
        check_state_scope(state, index)
    write_json(root / 'state.json', state)
    return view(root, state)


def load_state(root):
    from season_scope import project_index, check_state_scope, verify_previous
    manifest = verify_source(root)
    state = read_json(root / 'state.json')
    require(state.get('version') == 'series-workspace-v1', '未知工作状态版本')
    require(manifest['sha256'] == state['sourceHash'], '源版本变化')
    require(sha(root / 'source-index.json') == state['sourceIndexHash'], '源索引变化；拒绝套用旧状态')
    check_state_scope(state, project_index(root / 'source-index.json'))
    prior = state.get('continuationEvidence')
    actual = verify_previous(root, state.get('productionScope'),
                             prior.get('path') if isinstance(prior, dict) else None,
                             prior.get('sha256') if isinstance(prior, dict) else None)
    require(prior == actual, '上季闭季证据与当前续季派发不一致')
    return state


def previous(state, n):
    i = state['sourceOrder'].index(n)
    predecessor = state['sourceOrder'][i - 1] if i else None
    return predecessor if predecessor in state['scope'] else None


def completed_valid(root, ep):
    completion = ep.get('completion')
    return (ep['status'] == 'complete' and bool(completion)
            and all(valid_record(root, r) for r in completion['files']))


def boundary(root, state, n):
    upstream = previous(state, n)
    if upstream is None:
        return 'NOT_IN_SCOPE'
    ep = state['episodes'][str(n)]
    prev = state['episodes'][str(upstream)]
    if not completed_valid(root, prev):
        return 'PRELIMINARY'
    record = ep.get('boundaryRecord')
    if not record:
        return 'ACTUAL_PENDING_REVIEW'
    if not completed_valid(root, ep):
        return 'ACTUAL_PENDING_REVIEW'
    hashes_match = (record['upstreamHash'] == prev['completion']['adopted']['sha256']
                    and record['currentHash'] == ep['completion']['adopted']['sha256'])
    if not hashes_match or (record.get('evidence') and not valid_record(root, record['evidence'])):
        return 'STALE'
    return 'ACTUAL_RECONCILED'


def view(root, state):
    result = json.loads(json.dumps(state))
    for n in state['scope']:
        ep = result['episodes'][str(n)]
        ep['boundary'] = boundary(root, state, n)
        ep['evidenceValid'] = completed_valid(root, state['episodes'][str(n)])
    result['textComplete'] = all(ep['evidenceValid'] for ep in result['episodes'].values())
    result['readyForDelivery'] = result['textComplete'] and all(
        ep['boundary'] in ('NOT_IN_SCOPE', 'ACTUAL_RECONCILED') for ep in result['episodes'].values())
    return result


def compact_view(result):
    """Keep routine tool responses bounded; full evidence remains on disk."""
    if 'episodes' not in result:
        return result
    episodes = result['episodes']
    counts = {status: sum(e['status'] == status for e in episodes.values())
              for status in ('pending', 'in_flight', 'blocked', 'complete')}
    boundaries = {}
    for ep in episodes.values():
        b = ep['boundary']
        boundaries[b] = boundaries.get(b, 0) + 1
    next_items = []
    for group in result['groups']:
        n = next((n for n in group['episodes'] if not episodes[str(n)]['evidenceValid']), None)
        next_items.append({'group': group['id'], 'episode': n,
                           'status': episodes[str(n)]['status'] if n is not None else 'complete'})
    dispatch = result.get('dispatch')
    return {'version': result['version'], 'sourceHash': result['sourceHash'],
            'sourceIndexHash': result['sourceIndexHash'], 'scopeCount': len(result['scope']),
            'counts': counts, 'boundaries': boundaries, 'nextByGroup': next_items,
            'textComplete': result['textComplete'], 'readyForDelivery': result['readyForDelivery'],
            'dispatch': ({k: v for k, v in dispatch.items() if k != 'workers'} if dispatch else None),
            'workerCount': len(dispatch.get('workers', {})) if dispatch else None,
            'active': [{'episode': int(n), 'owner': e.get('owner')} for n, e in episodes.items()
                       if e['status'] == 'in_flight'],
            'fullStatePath': 'state.json', 'usage': result.get('usage'), 'cost': result.get('cost')}


def check_worker(state, owner):
    dispatch = state.get('dispatch')
    if not dispatch:  # Existing v1 evidence is never silently migrated.
        return
    require(owner not in {dispatch['coordinator'], '/root', 'root', 'main', 'coordinator'},
            '主控只统筹和审核，不得领取创作；请派子智能体')
    active = [e for e in state['episodes'].values() if e['status'] == 'in_flight']
    require(not any(e.get('owner') == owner for e in active), '同一执行者仍有执行中的集')
    require(len(active) < dispatch['activeSlots'] - 1, '执行槽已满；须为主控保留一个槽')
    worker = dispatch['workers'].get(owner, {})
    reason = worker.get('rotationReason')
    require(not reason, '须在已保存边界轮换新智能体: ' + str(reason))
    require(len(worker.get('claims', [])) < dispatch['batchSize'], '本执行者已达批次上限；须轮换新智能体')


def observe(root, state, owner, evidence):
    dispatch = state.get('dispatch')
    require(bool(dispatch), '旧工作区无调度策略；保留旧证据，不自动迁移')
    require(owner in dispatch['workers'], '尚未登记该执行者')
    record = file_record(root, evidence)
    data = read_json(root / record['path'])
    require(data.get('version') == 'series-context-observation-v1' and data.get('owner') == owner,
            '上下文观察版本或执行者不匹配')
    tokens, compactions = data.get('inputTokens'), data.get('compactions')
    require(tokens is None or (type(tokens) is int and tokens > 0), '未知输入用null，不填0；已知输入须为正整数')
    require(type(compactions) is int and compactions >= 0, 'compactions须为非负整数')
    require(isinstance(data.get('basis'), str) and bool(data['basis'].strip()), '须记录宿主观察依据')
    worker = dispatch['workers'][owner]
    worker.setdefault('observations', []).append({'evidence': record, 'inputTokens': tokens, 'compactions': compactions})
    if compactions:
        worker['rotationReason'] = '宿主已报告上下文压缩'
    elif tokens is not None and tokens >= dispatch['inputTokenLimit']:
        worker['rotationReason'] = '实际输入达到上下文阈值'
    return {'owner': owner, 'rotationRequired': bool(worker.get('rotationReason')),
            'reason': worker.get('rotationReason'), 'inputTokens': tokens,
            'note': '仅登记观察；不压缩/清空宿主上下文，不中断当前已领取集'}


def claim(root, state, group_id, owner):
    require(bool(owner.strip()), 'owner 不得为空')
    group = next((g for g in state['groups'] if g['id'] == group_id), None)
    require(group is not None, '组不存在')
    require(not any(state['episodes'][str(n)]['status'] == 'in_flight' for n in group['episodes']),
            '本组仍有执行中的集；须先核查并显式恢复，不能重叠领取')
    for n in group['episodes']:
        ep = state['episodes'][str(n)]
        if ep['status'] == 'complete':
            require(completed_valid(root, ep), '已完成稿证据变化；先显式 resume 与返修')
            if ep.get('boundaryRecord') or ep.get('claimedUpstreamHash'):
                require(boundary(root, state, n) == 'ACTUAL_RECONCILED',
                        '已绑定的实际前稿或边界证据失效；先核对/返修并 reconcile 第%s集' % n)
            continue
        require(ep['status'] == 'pending', '本组当前集已占用或阻塞；不会自动重派')
        check_worker(state, owner)
        ep.update(status='in_flight', owner=owner)
        if state.get('dispatch'):
            worker = state['dispatch']['workers'].setdefault(owner, {'claims': [], 'observations': []})
            worker['claims'].append(n)
        prev = previous(state, n)
        if prev is not None and state['episodes'][str(prev)]['group'] == group_id:
            ep['claimedUpstreamHash'] = state['episodes'][str(prev)]['completion']['adopted']['sha256']
        return {'episode': n, 'group': group_id, 'owner': owner, 'boundary': boundary(root, state, n)}
    return {'episode': None, 'group': group_id, 'reusedCompleted': True}


def complete(root, state, n, owner, evidence):
    ep = state['episodes'][str(n)]
    require(ep['status'] == 'in_flight' and ep.get('owner') == owner, '只能完成本人已领取的集')
    record = file_record(root, evidence)
    data = read_json(root / record['path'])
    require(data.get('version') == 'series-episode-evidence-v1' and data.get('episode') == n, '完成证据版本或集号不匹配')
    require(data.get('sourceHash') == state['sourceHash'] and data.get('sourceIndexHash') == state['sourceIndexHash'], '完成证据源/索引 hash 不匹配')
    source = next(e for e in read_json(root / 'source-index.json')['episodes'] if e['number'] == n)
    require(data.get('scenes') == [s['id'] for s in source['scenes']], '完成证据须覆盖本集全部场并保留源顺序')
    require(data.get('reviewMode') in ('independent', 'self'), '须如实记录 reviewMode')
    artifacts = data.get('artifacts', {})
    files = [record]
    for stage in STAGES:
        paths = artifacts.get(stage)
        paths = [paths] if isinstance(paths, str) else paths
        require(isinstance(paths, list) and bool(paths), '缺少阶段文件证据: ' + stage)
        files.extend(file_record(root, p) for p in paths)
    adopted = file_record(root, data.get('adopted', ''))
    files.append(adopted)
    if ep.get('claimedUpstreamHash'):
        entry = data.get('entryReview')
        require(isinstance(entry, dict)
                and entry.get('upstreamHash') == ep['claimedUpstreamHash']
                and isinstance(entry.get('notes'), str) and bool(entry['notes'].strip()),
                '组内续作须提供 entryReview 的实际前集 hash 与入口核对说明')
        prev = state['episodes'][str(previous(state, n))]
        require(completed_valid(root, prev)
                and prev['completion']['adopted']['sha256'] == ep['claimedUpstreamHash'],
                '领取后前集采用稿变化，需重新读取实际稿并恢复/返修')
    ep.update(status='complete', completion={'evidence': record, 'files': files, 'adopted': adopted, 'reviewMode': data['reviewMode']})
    if ep.get('claimedUpstreamHash'):
        ep['boundaryRecord'] = {'upstreamHash': ep['claimedUpstreamHash'], 'currentHash': adopted['sha256'], 'basis': 'entry_review_evidence', 'evidence': record}
    return view(root, state)


def recover_or_block(root, state, n, command, evidence, owner=None):
    ep = state['episodes'][str(n)]
    record = file_record(root, evidence)
    if command == 'block':
        require(ep['status'] == 'in_flight' and ep.get('owner') == owner, '只能阻塞本人已领取的集')
        ep.update(status='blocked', blockedEvidence=record)
    else:
        require(ep['status'] in ('in_flight', 'blocked', 'complete'), '只回收已占用、阻塞或待返修的集')
        require(not any(key != str(n) and other['group'] == ep['group'] and other['status'] == 'in_flight'
                        for key, other in state['episodes'].items()),
                '同组另有执行中的集；先核查并显式恢复该集，再返修前集')
        state['history'].append({'episode': n, 'prior': json.loads(json.dumps(ep)), 'resumeEvidence': record,
                                 'sourceHash': state['sourceHash'], 'sourceIndexHash': state['sourceIndexHash']})
        state['episodes'][str(n)] = {'status': 'pending', 'group': ep['group']}
    return view(root, state)


def reconcile(root, state, n, evidence):
    upstream = previous(state, n)
    require(upstream is not None, '前集不在范围，无需 reconcile')
    ep = state['episodes'][str(n)]
    prev = state['episodes'][str(upstream)]
    require(completed_valid(root, ep) and completed_valid(root, prev), '须有双方未变更的实际完成稿')
    record = file_record(root, evidence)
    data = read_json(root / record['path'])
    require(data.get('version') == 'series-boundary-v1' and data.get('upstream') == upstream
            and data.get('current') == n and data.get('outcome') == 'pass'
            and isinstance(data.get('notes'), str) and bool(data['notes'].strip()), '边界核对报告不完整')
    require(data.get('upstreamHash') == prev['completion']['adopted']['sha256']
            and data.get('currentHash') == ep['completion']['adopted']['sha256'], '边界核对 hash 与双方采用稿不匹配')
    ep['boundaryRecord'] = {'upstreamHash': data['upstreamHash'], 'currentHash': data['currentHash'], 'evidence': record}
    return view(root, state)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    p = commands.add_parser('extract')
    p.add_argument('--source', required=True, type=Path)
    p.add_argument('--output-dir', required=True, type=Path)
    for command in ('init', 'status', 'claim', 'complete', 'block', 'resume', 'reconcile', 'observe'):
        p = commands.add_parser(command)
        p.add_argument('--workdir', required=True, type=Path)
        verbosity = p.add_mutually_exclusive_group()
        verbosity.add_argument('--compact', action='store_true', help='仅返回计数、活动任务和下个就绪项')
        verbosity.add_argument('--full', action='store_true', help='显式读取完整状态；常规派发勿用')
        if command == 'init':
            p.add_argument('--index', required=True, type=Path)
            p.add_argument('--workers', type=int, help='连续逻辑队列数1～4，默认min(4,活动槽-1)')
            p.add_argument('--active-slots', type=int, default=4, help='宿主实际活动槽总数，包含主控')
            p.add_argument('--batch-size', type=int, default=2, help='同一执行者最多领取1～3集，默认2')
            p.add_argument('--coordinator', default='/root', help='主控实际身份，不得领取创作')
            p.add_argument('--input-limit', type=int, default=60000, help='实测输入Token轮换阈值；未知不估零')
            p.add_argument('--episodes', help='仅指定范围，逗号分隔整数集号')
            p.add_argument('--season-scope', type=Path, help='已锁定季范围；只派发选季内集场')
            p.add_argument('--previous-handoff', type=Path, help='续季必需：复制在本JOB内的上季closed包；包父目录为对接根')
        if command == 'claim':
            p.add_argument('--group', type=int, required=True)
        if command in ('claim', 'complete', 'block', 'observe'):
            p.add_argument('--owner', required=True)
        if command in ('complete', 'block', 'resume', 'reconcile'):
            p.add_argument('--episode', type=int, required=True)
            p.add_argument('--evidence', required=True)
        if command == 'observe':
            p.add_argument('--evidence', required=True)
    args = parser.parse_args()
    try:
        if args.command == 'extract':
            result = extract(args.source.resolve(), args.output_dir.resolve())
        else:
            root = args.workdir.resolve()
            with locked(root):
                if args.command == 'init':
                    selection = [int(n) for n in args.episodes.split(',')] if args.episodes else None
                    result = init(root, args.index.resolve(), args.workers, selection,
                                  args.active_slots, args.batch_size, args.coordinator, args.input_limit, args.season_scope, args.previous_handoff)
                else:
                    state = load_state(root)
                    if hasattr(args, 'episode'):
                        require(str(args.episode) in state['episodes'], '集号不在本次范围')
                    if args.command == 'status':
                        result = view(root, state)
                    elif args.command == 'claim':
                        result = claim(root, state, args.group, args.owner)
                    elif args.command == 'complete':
                        result = complete(root, state, args.episode, args.owner, args.evidence)
                    elif args.command in ('block', 'resume'):
                        result = recover_or_block(root, state, args.episode, args.command, args.evidence, getattr(args, 'owner', None))
                    elif args.command == 'observe':
                        result = observe(root, state, args.owner, args.evidence)
                    else:
                        result = reconcile(root, state, args.episode, args.evidence)
                    if args.command != 'status':
                        write_json(root / 'state.json', state)
        if args.command != 'extract' and (args.compact or (not args.full and result.get('dispatch'))):
            result = compact_view(result)
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0
    except (OSError, ValueError, KeyError, TypeError, ET.ParseError, zipfile.BadZipFile) as error:
        print('workspace: ' + str(error), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
