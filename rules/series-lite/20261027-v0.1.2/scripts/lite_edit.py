#!/usr/bin/env python3
"""Preflight frozen settings, or compose/patch creative fields in a single episode.

All file arguments except --workdir are JOB-relative. Hashes cover raw file bytes.
compose response: exactly {plan, selection, draft}; hash supplied separately.
patch document: {baseSha256, operations: [{op: "set", path, expected, value}]}.
Only existing /episodes/0/{plan,selection,draft} paths can be patched. Replace a
complete continuous unit or units array for deletion; there is no remove/append.
JSON values and Decimal precision survive; changed output formatting may differ.
This is mechanical assembly, NOT source/content validation or adoption. Afterwards
rebuild the matching job packet and run episode_execution.py run for the full draft.
"""
import argparse
from copy import deepcopy
import json
import re
import sys

from context_packet import (PacketError, canonical, fields, inside, job_path,
                            parse_json, project, read_file, sha256)
from visual_baseline import validate_visual_baseline


CREATIVE = {'plan', 'selection', 'draft'}


def need(condition, message):
    if not condition:
        raise PacketError(message)


def load_base(root, path, expected_hash):
    need(isinstance(expected_hash, str) and re.fullmatch('[0-9a-f]{64}', expected_hash),
         'base SHA-256 must be 64 lowercase hexadecimal characters')
    raw = read_file(root, path)
    need(sha256(raw) == expected_hash, 'base SHA-256 mismatch; rebuild from the current base')
    base = parse_json(raw, path)
    need(isinstance(base, dict) and base.get('version') == 'series-storyboard-bundle-v1',
         'base must be an existing storyboard bundle')
    episodes = base.get('episodes')
    need(isinstance(episodes, list) and len(episodes) == 1 and isinstance(episodes[0], dict),
         'base must contain exactly one episode')
    episode = episodes[0]
    need(type(episode.get('number')) is int, 'base episode number must be an integer')
    for key, kind in [('assets', list), ('scope', list)]:
        need(isinstance(base.get(key), kind), 'base missing frozen field: ' + key)
    for key, kind in [('source', dict), ('settings', dict), ('scenes', list)]:
        need(isinstance(episode.get(key), kind), 'base missing frozen episode field: ' + key)
    return base, raw


def pointer_tokens(path):
    need(isinstance(path, str) and path.startswith('/'), 'path must be a JSON Pointer')
    need(re.search('~(?![01])', path) is None, 'invalid JSON Pointer escape')
    tokens = tuple(token.replace('~1', '/').replace('~0', '~')
                   for token in path[1:].split('/'))
    need(len(tokens) >= 3 and tokens[:2] == ('episodes', '0') and tokens[2] in CREATIVE,
         'patch may only replace /episodes/0/{plan,selection,draft}')
    return tokens


def compose(base, response):
    fields(response, CREATIVE, set(), 'creative response')
    need(all(isinstance(response[key], dict) for key in CREATIVE),
         'plan, selection and draft must be JSON objects')
    result = deepcopy(base)
    for key in CREATIVE:
        result['episodes'][0][key] = deepcopy(response[key])
    return result


def patch(base, request):
    fields(request, {'baseSha256', 'operations'}, set(), 'patch document')
    need(isinstance(request['operations'], list), 'operations must be an array')
    paths = []
    for operation in request['operations']:
        fields(operation, {'op', 'path', 'expected', 'value'}, set(), 'set operation')
        need(operation['op'] == 'set', 'only set operations are supported')
        tokens = pointer_tokens(operation['path'])
        need(not any(tokens[:len(other)] == other or other[:len(tokens)] == tokens
                     for other in paths), 'overlapping patch operations')
        paths.append(tokens)
        old = project(base, operation['path'])
        need(canonical(old) == canonical(operation['expected']),
             'expected value mismatch at patch operation ' + str(len(paths)))
    result = deepcopy(base)
    for operation, tokens in zip(request['operations'], paths):
        # project() above already rejects nonexistent keys, noncanonical indices,
        # out-of-bounds access, and descent through scalars before any mutation.
        parent = result
        for key in tokens[:-1]:
            parent = parent[int(key)] if isinstance(parent, list) else parent[key]
        key = int(tokens[-1]) if isinstance(parent, list) else tokens[-1]
        parent[key] = deepcopy(operation['value'])
    return result


def write_result(root, output, base_path, expected_hash, base, base_raw, result):
    target = inside(root, output)
    need(not target.exists() and not (root / output).is_symlink(),
         'output already exists; refusing overwrite')
    changed = canonical(base) != canonical(result)
    raw = canonical(result) if changed else base_raw
    need(sha256(read_file(root, base_path)) == expected_hash, 'base changed during edit')
    target.parent.mkdir(parents=True, exist_ok=True)
    need(inside(root, output) == target, 'output path changed during edit')
    with target.open('xb') as stream:
        stream.write(raw)
    return {'path': str(target), 'sha256': sha256(raw), 'baseSha256': expected_hash,
            'bytes': len(raw), 'changed': changed, 'validation': 'not-run'}


def preflight(root, base_path):
    """Catch known setup mistakes before author dispatch; not draft validation."""
    raw = read_file(root, base_path)
    digest = sha256(raw)
    base, _ = load_base(root, base_path, digest)
    need(base.get('exportProfile') == 'lingmo-v7', 'preflight: lingmo-v7 is required')
    need(base.get('generationTextVersion') == 'voice-delivery-v1',
         'preflight: voice-delivery-v1 is required')
    need(isinstance(base.get('sourceLockHash'), str)
         and re.fullmatch('[0-9a-f]{64}', base['sourceLockHash']),
         'preflight: independent sourceLockHash is required')
    settings = base['episodes'][0]['settings']
    for key in ('visualStyle', 'prohibitions'):
        need(isinstance(settings.get(key), str) and settings[key].strip(),
             'preflight: settings.' + key + ' must be a nonempty string, not an array')
    need(type(settings.get('maxUnitSeconds')) is int and settings['maxUnitSeconds'] in (15, 30),
         'preflight: maxUnitSeconds must be 15 or 30')
    need(settings.get('fantasyMode') in ('off', 'recommended', 'all', 'review'),
         'preflight: a valid fantasyMode is required')
    try:
        validate_visual_baseline(settings.get('visualBaseline'))
    except ValueError as error:
        raise PacketError('preflight: ' + str(error)) from None
    need(settings['visualStyle'] == settings['visualBaseline']['visualStyle'],
         'preflight: visualStyle differs from the adopted baseline')
    return {'path': str(inside(root, base_path)), 'sha256': digest,
            'validation': 'frozen-setup-fields-only',
            'next': 'source-lock contents, registry, completed draft and semantic review still required'}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    check = commands.add_parser('preflight', help='read-only setup field check before author dispatch')
    check.add_argument('--workdir', required=True)
    check.add_argument('--base', required=True)
    for name in ('compose', 'patch'):
        command = commands.add_parser(name, help='mechanically assemble; full validation still required')
        command.add_argument('--workdir', required=True)
        command.add_argument('--base', required=True, help='existing frozen single-episode bundle')
        command.add_argument('--output', required=True, help='new JOB-relative output; never overwritten')
        if name == 'compose':
            command.add_argument('--response', required=True, help='JSON with only plan, selection, draft')
            command.add_argument('--expected-base-sha256', required=True)
        else:
            command.add_argument('--patch', required=True, help='baseSha256 plus set operations JSON')
    args = parser.parse_args(argv)
    try:
        root = job_path(args.workdir)
        if args.command == 'preflight':
            print(json.dumps(preflight(root, args.base), ensure_ascii=False))
            return 0
        input_path = args.response if args.command == 'compose' else args.patch
        request = parse_json(read_file(root, input_path), input_path)
        if args.command == 'compose':
            expected_hash = args.expected_base_sha256
        else:
            fields(request, {'baseSha256', 'operations'}, set(), 'patch document')
            expected_hash = request['baseSha256']
        base, raw = load_base(root, args.base, expected_hash)
        result = compose(base, request) if args.command == 'compose' else patch(base, request)
        summary = write_result(root, args.output, args.base, expected_hash, base, raw, result)
        print(json.dumps(summary, ensure_ascii=False))
        return 0
    except (PacketError, OSError, RuntimeError) as error:
        print('lite_edit: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
