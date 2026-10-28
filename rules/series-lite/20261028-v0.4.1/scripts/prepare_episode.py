#!/usr/bin/env python3
"""Prepare a role's exact inputs and reading view in one local call; no model calls.

Use an existing single-episode base/candidate and adopted registry. Select declared
scene/draft bindings and their design dependencies, never infer story facts. Extra
effective direction, context or issues may be referenced without rewriting them.
"""
import argparse
import json
import sys

from context_packet import PacketError, canonical, inside, job_path, parse_json, project, read_file, sha256
import job_packet
from lite_edit import preflight, load_base
from packet_view import build_view


def need(ok, message):
    if not ok:
        raise PacketError(message)


def select_assets(root, registry_path, base, role):
    directory = inside(root, registry_path)
    head_path = (directory / 'current.json').relative_to(root).as_posix()
    head = parse_json(read_file(root, head_path), head_path)
    need(head.get('version') == 'series-assets-head-v1' and type(head.get('revision')) is int,
         'invalid registry head')
    snapshot = (directory / ('v%06d.json' % head['revision'])).relative_to(root).as_posix()
    raw = read_file(root, snapshot)
    need(sha256(raw) == head['sha256'], 'registry snapshot changed')
    registry = parse_json(raw, snapshot)
    need(registry.get('version') == 'series-assets-v1' and registry.get('id') == head['id']
         and registry.get('revision') == head['revision'], 'registry identity mismatch')
    need(registry['sourceHash'] == base['sourceHash']
         and registry['id'] == base['assetRegistry']['id'], 'base and registry differ')
    ids = {a['id']: a for a in registry['assets']}
    by_key = {a['key']: a for a in base['assets']}
    episode = base['episodes'][0]
    owners = episode['scenes'] if role == 'author' else episode['draft'].get('units', [])
    need(owners, 'missing scene/draft bindings')
    keys = list(dict.fromkeys(key for owner in owners for key in owner['bindings']))
    need(keys, 'empty scene/draft bindings')
    chosen, visiting = {}, set()

    def add(asset_id, revision):
        need(asset_id not in visiting, 'cyclic asset dependency: ' + str(asset_id))
        asset = ids.get(asset_id)
        need(asset is not None and asset.get('revision') == revision,
             'missing or changed required asset: ' + str(asset_id))
        if asset_id in chosen:
            return
        need(asset.get('design', {}).get('state') in ('TEXT_ADOPTED', 'REFERENCE_ADOPTED'),
             'required design is not adopted: ' + asset_id)
        visiting.add(asset_id)
        for dependency in asset['design'].get('dependencies', []):
            add(dependency['assetId'], dependency['assetRevision'])
        visiting.remove(asset_id)
        chosen[asset_id] = asset  # Preserve all row fields; never guess away constraints.

    for key in keys:
        need(key in by_key, 'binding absent from base assets: ' + str(key))
        declared = by_key[key]
        add(declared.get('assetId'), declared.get('assetRevision'))
        need(ids[declared['assetId']]['key'] == key, 'asset identity/key mismatch')
    # Requests, image designs and full registry history are machine records, not
    # episode inputs. Complete selected rows (including unknown constraints) stay.
    return {'assets': list(chosen.values())}, {'path': snapshot, 'sha256': sha256(raw)}


def prepare(workdir, base_path, registry_path, role, output, *, context=None,
            context_pointer=None, direction=None, direction_pointer=None,
            issues=None, issues_pointer=None, methods=(), contracts=(),
            max_bytes=job_packet.DEFAULT_MAX_BYTES):
    root = job_path(workdir)
    need(role in job_packet.ROLES, 'invalid role')
    destination = inside(root, output)
    need(not destination.exists(), 'output exists; use a new version directory')
    preflight(root, base_path)
    base, raw = load_base(root, base_path, sha256(read_file(root, base_path)))
    episode = base['episodes'][0]
    selected, snapshot = select_assets(root, registry_path, base, role)
    settings = {k: v for k, v in episode['settings'].items()
                if k not in ('visualBaseline', 'visualStyle')}
    materials = {'direction': settings, 'assets': selected,
                 'origin': {'base': {'path': base_path, 'sha256': sha256(raw)}, 'registry': snapshot}}
    relative = lambda name: (destination / name).relative_to(root).as_posix()
    inputs = [{'kind': 'source', 'path': base_path, 'pointer': '/episodes/0/source'},
              {'kind': 'scenes', 'path': base_path, 'pointer': '/episodes/0/scenes'},
              {'kind': 'visual-baseline', 'path': base_path, 'pointer': '/episodes/0/settings/visualBaseline'},
              {'kind': 'direction', 'path': relative('materials.json'), 'pointer': '/direction'},
              {'kind': 'assets', 'path': relative('materials.json'), 'pointer': '/assets'}]
    for kind, path, pointer in [('state', context, context_pointer),
                                 ('direction', direction, direction_pointer),
                                 ('issues', issues, issues_pointer)]:
        need(path is not None or pointer is None, kind + ' pointer requires a file')
        if path is not None:
            data = read_file(root, path)
            if pointer is not None:
                project(parse_json(data, path), pointer)
            inputs.append({'kind': kind, 'path': path, **({'pointer': pointer} if pointer is not None else {})})
    if role != 'author':
        for kind in ('plan', 'selection', 'draft'):
            need(isinstance(episode.get(kind), dict) and bool(episode[kind]), 'missing creative field: ' + kind)
            inputs.append({'kind': kind, 'path': base_path, 'pointer': '/episodes/0/' + kind})
    request = {'version': job_packet.REQUEST_VERSION, 'episode': episode['number'],
               'role': role, 'inputs': inputs, 'methods': list(methods), 'contracts': list(contracts)}
    job_packet.validate_request(request)
    job_packet.scope_binding(root, episode['number'])
    destination.mkdir(parents=True, exist_ok=False)
    for name, value in [('materials.json', materials), ('request.json', request)]:
        with inside(root, relative(name)).open('xb') as out:
            out.write(canonical(value))
    job_packet.build(root, relative('request.json'), relative('packet.json'), max_bytes)
    view = build_view(root, relative('packet.json'), relative('read.md'))
    return {'episode': episode['number'], 'role': role, 'packet': relative('packet.json'),
            'read': relative('read.md'), 'baseSha256': sha256(raw),
            'selectedAssets': len(selected['assets']), 'readingBytes': view['bytes'],
            'validation': 'mechanical preparation only; no content review or asset completeness attested'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('workdir', 'base', 'registry', 'output'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--role', choices=sorted(job_packet.ROLES), default='author')
    for name in ('context', 'direction', 'issues'):
        parser.add_argument('--' + name)
        parser.add_argument('--' + name + '-pointer')
    parser.add_argument('--method', action='append', default=[])
    parser.add_argument('--contract', action='append', default=[])
    parser.add_argument('--max-bytes', type=int, default=job_packet.DEFAULT_MAX_BYTES)
    args = parser.parse_args()
    try:
        result = prepare(args.workdir, args.base, args.registry, args.role, args.output,
                         context=args.context, context_pointer=args.context_pointer,
                         direction=args.direction, direction_pointer=args.direction_pointer,
                         issues=args.issues, issues_pointer=args.issues_pointer,
                         methods=args.method, contracts=args.contract, max_bytes=args.max_bytes)
        print(json.dumps(result, ensure_ascii=False))
        return 0
    except (PacketError, OSError, KeyError, TypeError) as error:
        print('prepare_episode: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
