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
from decimal import Decimal
import json
import re
import sys

from context_packet import (PacketError, canonical, fields, inside, job_path,
                            parse_json, project, read_file, sha256)
from visual_baseline import validate_visual_baseline


CREATIVE = {'plan', 'selection', 'draft'}
PLAN_VERSION = 'performance-core-v4-plan'
UNIT_PACKING_POLICY = 'event-first-v1'


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


def nonempty(value):
    return isinstance(value, str) and bool(value.strip())


def derive_plan_bookkeeping(episode):
    """Index a completed draft, never infer prior planning or creative content.

    Stable source IDs are required, as by storyboard.mjs checkBundle. The
    generated provisionalUnits are retrospective exact-duration indices, not
    evidence that capacity planning happened before the draft was written.
    """
    plan, draft = episode['plan'], episode['draft']
    supplied = [key in plan for key in ('workPackages', 'provisionalUnits')]
    need(supplied[0] == supplied[1],
         'derive-plan refuses mixed missing workPackages/provisionalUnits')
    if all(supplied):
        return False
    need(plan.get('schemaVersion', PLAN_VERSION) == PLAN_VERSION,
         'derive-plan: unknown plan schemaVersion')
    need(plan.get('unitPackingPolicy', UNIT_PACKING_POLICY) == UNIT_PACKING_POLICY,
         'derive-plan: unknown unitPackingPolicy')
    need(nonempty(plan.get('emotionArc')), 'derive-plan requires authored emotionArc')
    for key in ('entryFacts', 'fantasyCandidates'):
        need(isinstance(plan.get(key), list), 'derive-plan requires authored ' + key)
    need(isinstance(episode['selection'].get('selectedIds'), list),
         'derive-plan requires authored selection.selectedIds')
    source = episode['source']
    need(nonempty(source.get('key')), 'derive-plan requires source.key')
    rows = source.get('sourceRows')
    contexts = source.get('contextRows', [])
    need(isinstance(rows, list) and rows and isinstance(contexts, list),
         'derive-plan requires sourceRows and contextRows arrays')

    def stable_ids(items):
        ids = []
        for row in items:
            need(isinstance(row, dict) and isinstance(row.get('text'), str),
                 'derive-plan: invalid frozen source row')
            identity = row.get('id')
            if identity is None and nonempty(row.get('sourceId')):
                identity = source['key'] + ':' + row['sourceId']
            if identity is None and type(row.get('line')) is int and row['line'] >= 0:
                identity = source['key'] + ':L' + str(row['line']).zfill(4)
            need(nonempty(identity), 'derive-plan requires stable frozen source IDs')
            ids.append(identity)
        return ids

    source_ids, context_ids = stable_ids(rows), stable_ids(contexts)
    all_ids = source_ids + context_ids
    need(len(set(all_ids)) == len(all_ids), 'derive-plan: duplicate frozen source IDs')
    order = {identity: index for index, identity in enumerate(source_ids)}
    allowed_context = set(all_ids)

    def refs(value, allowed, label, required=True):
        need(isinstance(value, list) and (value or not required)
             and all(nonempty(ref) and ref in allowed for ref in value)
             and len(set(value)) == len(value),
             'derive-plan: empty, duplicate or unknown ' + label)
        return value

    events = plan.get('events')
    need(isinstance(events, list) and events, 'derive-plan requires authored events')
    event_map = {}
    for index, event in enumerate(events):
        need(isinstance(event, dict) and nonempty(event.get('id'))
             and event['id'] not in event_map
             and event['id'] not in ('__proto__', 'constructor', 'prototype')
             and nonempty(event.get('sceneKey')), 'derive-plan: invalid or duplicate event ID/sceneKey')
        boundary = event.get('boundaryBefore')
        need(isinstance(boundary, dict) and boundary.get('kind') in
             ('episode_start', 'continuous', 'new_event', 'time_space_jump')
             and nonempty(boundary.get('reason'))
             and ((index == 0) == (boundary['kind'] == 'episode_start')),
             'derive-plan requires authored event boundaryBefore')
        emotion = event.get('emotionContract')
        need(isinstance(emotion, dict)
             and all(nonempty(emotion.get(key)) for key in
                     ('subject', 'contextTone', 'audienceEffect', 'change', 'landing'))
             and isinstance(emotion.get('retain'), list) and emotion['retain']
             and all(nonempty(value) for value in emotion['retain']),
             'derive-plan requires complete authored emotionContract')
        refs(event.get('sourceRefs'), order, 'event sourceRefs')
        event_map[event['id']] = event

    facts = plan['entryFacts']
    fact_ids = set()
    for fact in facts:
        need(isinstance(fact, dict) and nonempty(fact.get('id'))
             and fact['id'] not in fact_ids, 'derive-plan: invalid or duplicate entryFact ID')
        fact_ids.add(fact['id'])
        refs(fact.get('sourceRefs'), allowed_context, 'entryFact sourceRefs')
    units = draft.get('units')
    need(isinstance(units, list) and units, 'derive-plan requires completed draft.units')
    unit_ids, shot_ids, provisional = set(), set(), []
    event_context = {identity: set() for identity in event_map}
    used_events = set()
    for unit in units:
        need(isinstance(unit, dict) and nonempty(unit.get('id'))
             and unit['id'] not in unit_ids, 'derive-plan: invalid or duplicate unit ID')
        unit_ids.add(unit['id'])
        event_id = unit.get('eventId')
        need(isinstance(event_id, str) and event_id in event_map,
             'derive-plan: unit missing eventId or references unknown eventId')
        used_events.add(event_id)
        shots = unit.get('shots')
        need(isinstance(shots, list) and shots, 'derive-plan requires completed unit shots')
        total_ticks, used_refs = 0, set()
        for shot in shots:
            need(isinstance(shot, dict) and nonempty(shot.get('id'))
                 and shot['id'] not in shot_ids, 'derive-plan: invalid or duplicate shot ID')
            shot_ids.add(shot['id'])
            used_refs.update(refs(shot.get('sourceRefs'), order, 'shot sourceRefs'))
            need(set(shot['sourceRefs']).issubset(event_map[event_id]['sourceRefs']),
                 'derive-plan: shot sourceRefs outside its event')
            event_context[event_id].update(refs(shot.get('contextRefs', []), allowed_context,
                                                'shot contextRefs', required=False))
            duration = shot.get('duration')
            need(type(duration) in (int, float, Decimal), 'derive-plan: invalid shot duration')
            duration = Decimal(str(duration))
            need(duration.is_finite() and duration > 0, 'derive-plan: invalid shot duration')
            numerator, denominator = duration.as_integer_ratio()
            ticks, remainder = divmod(numerator * 1000000000, denominator)
            need(not remainder and 0 < ticks <= 9007199254740991,
                 'derive-plan: duration exceeds exact 9-decimal/safe tick precision')
            total_ticks += ticks
        need(total_ticks <= 9007199254740991, 'derive-plan: unit duration exceeds safe tick precision')
        seconds = Decimal((str(total_ticks // 1000000000) + '.'
                           + str(total_ticks % 1000000000).zfill(9)).rstrip('0').rstrip('.'))
        provisional.append({'id': unit['id'], 'packageId': event_id,
                            'sourceRefs': sorted(used_refs, key=order.__getitem__),
                            'secondsRange': [seconds, seconds]})
    need(used_events == set(event_map), 'derive-plan: event has no completed unit')
    packages = []
    for event in events:
        context = event_context[event['id']]
        evidence = set(event['sourceRefs']) | context
        packages.append({'id': event['id'], 'eventId': event['id'],
                         'sourceRefs': deepcopy(event['sourceRefs']),
                         'emotionContract': deepcopy(event['emotionContract']),
                         'contextRefs': [ref for ref in all_ids if ref in context],
                         'entryFactIds': [fact['id'] for fact in facts
                                          if set(fact['sourceRefs']).issubset(evidence)],
                         'dependsOn': [], 'scenarioKeys': []})
    plan.setdefault('schemaVersion', PLAN_VERSION)
    plan.setdefault('unitPackingPolicy', UNIT_PACKING_POLICY)
    plan['workPackages'], plan['provisionalUnits'] = packages, provisional
    return True


def compose(base, response, derive_plan=False):
    fields(response, CREATIVE, set(), 'creative response')
    need(all(isinstance(response[key], dict) for key in CREATIVE),
         'plan, selection and draft must be JSON objects')
    result = deepcopy(base)
    for key in CREATIVE:
        result['episodes'][0][key] = deepcopy(response[key])
    if derive_plan:
        derive_plan_bookkeeping(result['episodes'][0])
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
    need(base.get('generationBoundaryVersion') == 'video-prompt-v1',
         'preflight: generationBoundaryVersion video-prompt-v1 is required')
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
            command.add_argument('--derive-plan', action='store_true',
                                 help='derive missing WP/PU bookkeeping from completed draft; not prior capacity planning')
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
        result = (compose(base, request, args.derive_plan) if args.command == 'compose'
                  else patch(base, request))
        summary = write_result(root, args.output, args.base, expected_hash, base, raw, result)
        if args.command == 'compose' and args.derive_plan:
            summary['derivedBookkeeping'] = 'workPackages' not in request['plan']
            if summary['derivedBookkeeping']:
                summary['bookkeepingBasis'] = 'completed-draft-index; exact shot totals; not prior capacity planning'
        print(json.dumps(summary, ensure_ascii=False))
        return 0
    except (PacketError, OSError, RuntimeError) as error:
        print('lite_edit: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
