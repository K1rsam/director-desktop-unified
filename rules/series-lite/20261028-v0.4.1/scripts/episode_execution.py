#!/usr/bin/env python3
"""Idempotent local episode validation/export. No model calls or creative decisions."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from context_packet import canonical, fields, inside, job_path, parse_json, read_file, sha256
from workspace import locked

SCRIPTS = Path(__file__).resolve().parent
REQUEST = 'series-episode-execution-v1'
RECEIPT = 'series-episode-adoption-v1'


def need(ok, message):
    if not ok:
        raise ValueError(message)


def read(root, path):
    return parse_json(read_file(root, path), path)


def record(root, path):
    return {'path': inside(root, path).relative_to(root).as_posix(),
            'sha256': sha256(read_file(root, path))}


def atomic(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix='.execution-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as out:
            out.write(canonical(value) + b'\n')
            out.flush()
            os.fsync(out.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def node(script, args):
    result = subprocess.run(['node', str(SCRIPTS / script), *map(str, args)],
                            capture_output=True, timeout=180)
    need(result.returncode == 0, script + ': ' + result.stderr.decode('utf-8', errors='replace').strip())
    return parse_json(result.stdout, script)


def packet_value(packet, binding):
    value = packet['blobs'][binding['blob']]
    return parse_json(value['value'].encode('utf-8'), binding['path']) if value['format'] == 'text' else value['value']


def asset_view(root, registry_path, bundle, cache=None):
    registry_dir = inside(root, registry_path)
    key = ('registry', str(registry_dir))
    loaded = cache.get(key) if cache is not None else None
    if loaded is None:
        head = read(root, (registry_dir / 'current.json').relative_to(root).as_posix())
        need(head.get('version') == 'series-assets-head-v1' and type(head.get('revision')) is int
             and head['revision'] > 0, 'invalid registry head')
        snapshot = registry_dir / ('v%06d.json' % head['revision'])
        snapshot_path = snapshot.relative_to(root).as_posix()
        raw = read_file(root, snapshot_path)
        need(sha256(raw) == head['sha256'], 'current registry snapshot changed')
        registry = parse_json(raw, snapshot_path)
        need(registry.get('version') == 'series-assets-v1' and registry['id'] == head['id']
             and registry['revision'] == head['revision'], 'registry identity mismatch')
        loaded = (registry, {'path': snapshot_path, 'sha256': head['sha256']})
        if cache is not None:
            cache[key] = loaded
    registry, snapshot = loaded
    ids = {a['id']: a for a in registry['assets']}
    projection = {a['key']: a.get('assetId') for a in bundle['assets']}
    wanted = set()
    def add(asset_id):
        if asset_id in wanted:
            return
        need(asset_id in ids, 'missing declared asset: ' + str(asset_id))
        wanted.add(asset_id)
        for dependency in ids[asset_id]['design'].get('dependencies', []):
            add(dependency['assetId'])
    for episode in bundle['episodes']:
        for unit in episode['draft']['units']:
            for key in unit['bindings']:
                add(projection.get(key))
    numbers = {e['number'] for e in bundle['episodes']}
    relevant = {'id': registry['id'], 'sourceHash': registry['sourceHash'],
                'importProfile': registry.get('importProfile'), 'productionScope': registry.get('productionScope'),
                'assets': [ids[key] for key in sorted(wanted)],
                'requests': [r for r in registry['requests'] if any(u['episode'] in numbers for u in r['uses'])]}
    return sha256(canonical(relevant)), snapshot


def inputs(root, manifest, historical_runtime=None, cache=None):
    from job_packet import load_verified, load_frozen
    request = read(root, manifest)
    fields(request, {'version', 'episode', 'packet', 'bundle', 'sourceLock', 'registry'},
           {'seasonScope'}, 'episode execution request')
    need(request['version'] == REQUEST, 'unknown execution version')
    packet = (load_verified if historical_runtime is None else load_frozen)(root, request['packet'])
    need(packet['episode'] == request['episode'] and packet['role'] in ('finisher', 'reviewer'),
         'adoption needs the current complete finisher/reviewer packet')
    bundle = read(root, request['bundle'])
    need(bundle.get('exportProfile') == 'lingmo-v7' and len(bundle.get('episodes', [])) == 1,
         'episode execution requires one complete Lingmo v7 episode')
    # New execution is strict; historical receipts use their frozen packet contract.
    if historical_runtime is None or "generationBoundaryVersion:'video-prompt-v1'" in packet['task']['contract']:
        need(bundle.get('generationBoundaryVersion') == 'video-prompt-v1',
             'generationBoundaryVersion: new packet candidates require video-prompt-v1')
    episode = bundle['episodes'][0]
    need(episode['number'] == request['episode'], 'bundle episode differs from packet')
    for kind in ('source', 'plan', 'selection'):
        matches = [packet_value(packet, b) for b in packet['bindings'] if b['kind'] == kind]
        need(len(matches) == 1 and canonical(matches[0]) == canonical(episode[kind]),
             kind + ': packet and candidate bundle must use the same complete value; rebuild after amendment')
    baselines = [packet_value(packet, b) for b in packet['bindings'] if b['kind'] == 'visual-baseline']
    if baselines:
        need(len(baselines) == 1
             and canonical(baselines[0]) == canonical(episode.get('settings', {}).get('visualBaseline'))
             and episode['settings'].get('visualStyle') == baselines[0]['visualStyle'],
             'visual-baseline: packet and candidate settings must preserve the same complete adopted value')
    elif historical_runtime is None:
        need(False, 'visual-baseline: new adoption requires the adopted project baseline in the packet')
    # Rules and every runtime module are part of the deterministic execution version.
    runtime = historical_runtime if historical_runtime is not None else {
        p.relative_to(SCRIPTS).as_posix(): sha256(p.read_bytes())
        for p in sorted(SCRIPTS.rglob('*')) if p.suffix in ('.py', '.mjs')}
    files = [record(root, manifest)] + [record(root, request[k]) for k in
             ('packet', 'bundle', 'sourceLock', 'seasonScope') if k in request]
    registry = inside(root, request['registry'])
    need(registry.is_dir(), 'registry must be an existing JOB directory')
    # New candidates receive the full existing Node registry validation. Historical
    # receipts compare the frozen relevant values, avoiding a process per episode
    # on every status read. Final production checks still use the current registry.
    if historical_runtime is None:
        node('execution-dependencies.mjs', [registry, inside(root, request['bundle'])])
    asset_fingerprint, snapshot = asset_view(root, request['registry'], bundle, cache)
    signature = {'files': files, 'inputFingerprint': packet['inputFingerprint'],
                 'runtime': runtime, 'assetFingerprint': asset_fingerprint}
    return request, signature, sha256(canonical(signature)), snapshot


def verify_adoption(root, path, episode=None, cache=None):
    root = job_path(root)
    receipt = read(root, path)
    need(receipt.get('version') == RECEIPT, 'unknown adoption receipt')
    copy = {k: v for k, v in receipt.items() if k != 'receiptSha256'}
    need(sha256(canonical(copy)) == receipt.get('receiptSha256'), 'adoption receipt changed')
    need(episode is None or episode == receipt['episode'], 'adoption episode differs')
    request, signature, fingerprint, _ = inputs(root, receipt['manifest'], receipt['inputs']['runtime'], cache)
    need(fingerprint == receipt['executionFingerprint'] and signature == receipt['inputs'],
         'adoption inputs changed; retain this candidate and create a new version')
    need(receipt['episode'] == request['episode'] and receipt['bundle'] == record(root, request['bundle']),
         'adoption bundle mismatch')
    for item in receipt['outputs']:
        need(record(root, item['path']) == item, 'adoption output changed: ' + item['path'])
    snapshot = receipt['registrySnapshot']
    need(record(root, snapshot['path']) == snapshot, 'adopted registry snapshot changed')
    need({'sync', 'check', 'export', 'verify'} == set(receipt['steps']), 'incomplete adoption steps')
    # Hashes prove local version consistency, never independent content review.
    return receipt


def run(root, manifest, output):
    root = job_path(root)
    run_dir = inside(root, output)
    state_file = run_dir / 'execution.json'
    if run_dir.exists() and not state_file.is_file():
        need(not any(run_dir.iterdir()), 'output directory must be new/empty or an existing execution run')
    run_dir.mkdir(parents=True, exist_ok=True)
    with locked(run_dir):
        receipt_path = run_dir / 'adoption.json'
        if receipt_path.exists():
            receipt = verify_adoption(root, receipt_path.relative_to(root).as_posix())
            need(receipt['manifest'] == manifest, 'output belongs to another execution request')
            return {'ok': True, 'reused': True, 'receipt': receipt_path.relative_to(root).as_posix(), 'stepsRun': []}
        request, signature, fingerprint, snapshot = inputs(root, manifest)
        state = parse_json(state_file.read_bytes(), 'execution state') if state_file.exists() else {
            'version': REQUEST, 'executionFingerprint': fingerprint, 'steps': {}, 'attempt': 0}
        need(state.get('version') == REQUEST, 'output directory has no valid execution journal')
        need(state['executionFingerprint'] == fingerprint,
             'execution inputs changed; use a new candidate/output directory')
        # Outputs live in their own directory; never overwrite any supplied data,
        # including rule-packet dependencies, even when resuming an existing run.
        packet = read(root, request['packet'])
        protected = [r['path'] for r in signature['files']]
        protected += [r['path'] for r in packet['bindings']]
        protected += [request['registry']]
        for key in ('review', 'requestOrigin'):
            if isinstance(packet.get(key), dict) and 'path' in packet[key]:
                protected.append(packet[key]['path'])
        need(not any(inside(root, path).is_relative_to(run_dir) for path in protected),
             'execution output directory overlaps an input; choose a new directory')
        def rel(path):
            return path.relative_to(root).as_posix()
        def save():
            atomic(state_file, state)
        def finish_step(name, result, paths=()):
            report = run_dir / (name + '.json')
            atomic(report, result)
            state['steps'][name] = [record(root, rel(p)) for p in (report, *paths)]
            save()
        # Never trust a journal when its referenced output has drifted.
        for outputs in state['steps'].values():
            for item in outputs:
                need(record(root, item['path']) == item, 'cached output changed: ' + item['path'])
        save()
        args = ['--input', inside(root, request['bundle']), '--source-lock', inside(root, request['sourceLock']),
                '--registry', inside(root, request['registry'])]
        if 'seasonScope' in request:
            args += ['--season-scope', inside(root, request['seasonScope'])]
        performed = []
        if 'sync' not in state['steps']:
            consumption = run_dir / 'consumption.json'
            if not consumption.exists():
                node('assets.mjs', ['sync', *args, '--output', consumption])
            else:
                # A process can stop after the exclusive output write but before journal save.
                prior = read(root, rel(consumption))
                need(prior.get('bundleSha256') == record(root, request['bundle'])['sha256'],
                     'interrupted consumption belongs to another bundle')
                old_snapshot = Path(prior['registrySnapshot']).resolve()
                need(old_snapshot.is_relative_to(root) and sha256(old_snapshot.read_bytes()) == prior['registrySha256'],
                     'interrupted consumption snapshot invalid')
            finish_step('sync', {'ok': True}, (consumption,))
            performed.append('sync')
        if 'check' not in state['steps']:
            finish_step('check', node('storyboard.mjs', ['check', *args]))
            performed.append('check')
        if 'export' not in state['steps']:
            preview = run_dir / ('export-' + str(state['attempt'])) / 'preview.md'
            catalog = Path(str(preview) + '.assets.json')
            if preview.exists() and catalog.exists():
                result = node('storyboard.mjs', ['verify', *args, '--output', preview])
            else:
                if preview.exists() or catalog.exists():
                    # Preserve interrupted fragments; never overwrite an existing export.
                    state['attempt'] += 1
                    save()
                    preview = run_dir / ('export-' + str(state['attempt'])) / 'preview.md'
                    catalog = Path(str(preview) + '.assets.json')
                result = node('storyboard.mjs', ['export', *args, '--output', preview])
            state['preview'] = rel(preview)
            finish_step('export', result, (preview, catalog))
            performed.append('export')
        if 'verify' not in state['steps']:
            finish_step('verify', node('storyboard.mjs', ['verify', *args, '--output', inside(root, state['preview'])]))
            performed.append('verify')
        _, current, now, _ = inputs(root, manifest)
        need(now == fingerprint and current == signature, 'inputs changed during execution; no adoption written')
        receipt = {'version': RECEIPT, 'episode': request['episode'], 'manifest': manifest,
                   'bundle': record(root, request['bundle']), 'executionFingerprint': fingerprint,
                   'inputs': signature, 'steps': list(state['steps']),
                   'registrySnapshot': snapshot,
                   'outputs': [item for group in state['steps'].values() for item in group],
                   'preview': state['preview'], 'contentReview': 'NOT_PROVEN'}
        receipt['receiptSha256'] = sha256(canonical(receipt))
        atomic(receipt_path, receipt)
        return {'ok': True, 'reused': False, 'receipt': rel(receipt_path), 'stepsRun': performed}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('run')
    p.add_argument('--workdir', required=True)
    p.add_argument('--manifest', required=True)
    p.add_argument('--output', required=True)
    p = sub.add_parser('verify')
    p.add_argument('--workdir', required=True)
    p.add_argument('--receipt', required=True)
    args = parser.parse_args()
    try:
        root = job_path(args.workdir)
        if args.command == 'run':
            result = run(root, args.manifest, args.output)
        else:
            receipt = verify_adoption(root, args.receipt)
            result = {'ok': True, 'episode': receipt['episode'], 'contentReview': 'NOT_PROVEN'}
        print(canonical(result).decode('utf-8'))
        return 0
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print('episode_execution: ' + str(error), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
