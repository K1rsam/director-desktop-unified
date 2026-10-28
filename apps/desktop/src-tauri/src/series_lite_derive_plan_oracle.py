"""Development-only v0.4.1 derive-plan oracle; never run by desktop clients.

Use only the pinned synthetic upstream fixtures and actual frozen Python code.
Outputs compare complete UTF-8 bytes, including source CRLF, exact decimals and
no-change original-file reuse. No production data, model, registry or media calls.
"""
from copy import deepcopy
from decimal import Decimal
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
BASE = HERE.parents[3] / 'rules/series-lite/20261028-v0.4.1'
sys.path.insert(0, str(BASE / 'scripts'))
sys.path.insert(0, str(BASE / 'tests'))
from context_packet import canonical, parse_json, sha256
from lite_edit import compose, load_base
from test_lite_derive_plan import DerivePlanTests


def main():
    cases = []

    def add(name, mutate=None, raw_mutate=None, expected_hash=None):
        fixture = DerivePlanTests()
        fixture.setUp()
        base, response = fixture.base, fixture.response
        if mutate:
            mutate(base, response)
        base_raw, response_raw = canonical(base), canonical(response)
        if raw_mutate:
            base_raw, response_raw = raw_mutate(base_raw, response_raw)
        expected = expected_hash or sha256(base_raw)
        case = {'name': name, 'base': base_raw.decode(), 'response': response_raw.decode(),
                'expectedHash': expected}
        try:
            # load_base's path IO and raw hash are represented explicitly here.
            if sha256(base_raw) != expected:
                raise ValueError('base SHA-256 mismatch')
            base = parse_json(base_raw, 'base')
            response = parse_json(response_raw, 'response')
            result = compose(base, response, derive_plan=True)
            changed = canonical(base) != canonical(result)
            output = canonical(result) if changed else base_raw
            case.update(ok=True, output=output.decode(), sha256=sha256(output),
                        changed=changed, derivedBookkeeping=not all(
                            k in response['plan'] for k in ('workPackages', 'provisionalUnits')))
        except (ValueError, KeyError, TypeError, ArithmeticError) as error:
            case.update(ok=False, error=str(error))
        cases.append(case)

    add('complete-event-contract-and-context')
    add('nanoseconds-source-order', lambda b, r: (
        r['draft']['units'][0]['shots'][0].update(duration=Decimal('0.100000001'),
            sourceRefs=list(reversed(r['plan']['events'][0]['sourceRefs']))),
        r['draft']['units'][0]['shots'][1].update(duration=Decimal('0.200000002'),
            sourceRefs=r['plan']['events'][0]['sourceRefs'][:1])))
    add('no-artificial-trailing-precision', lambda b, r: (
        r['draft']['units'][0]['shots'][0].update(duration=Decimal('6.1')),
        r['draft']['units'][0]['shots'][1].update(duration=Decimal('12.5'))))
    add('preserve-crlf-and-spaces', lambda b, r: (
        b['episodes'][0]['source']['sourceRows'][0].update(text='原行\r\n第二行  \r\n'),
        b['episodes'][0]['source']['contextRows'][0].update(text='前情\r\n不清洗\t  ')))
    add('derive-schema-defaults', lambda b, r: (
        r['plan'].pop('schemaVersion', None), r['plan'].pop('unitPackingPolicy', None)))
    add('explicit-plan-unchanged', lambda b, r: r['plan'].update(
        workPackages=[{'opaque': 'author owned'}], provisionalUnits=[{'opaque': 'author owned'}]))

    def unchanged(b, r):
        r['plan'].update(workPackages=[], provisionalUnits=[])
        b['episodes'][0].update(deepcopy(r))
    add('unchanged-preserves-original-byte-format', unchanged,
        lambda b, r: (b'\r\n  ' + b + b'\r\n', r))
    add('sourceId-fallback', lambda b, r: [
        (row.update(sourceId=row['id'].split(':', 1)[1]), row.pop('id'))
        for row in b['episodes'][0]['source']['sourceRows']])

    def line_ids(b, r):
        mapping = {}
        for i, row in enumerate(b['episodes'][0]['source']['sourceRows']):
            mapping[row.pop('id')] = 'E01:L' + str(i).zfill(4)
            row['line'] = i
        for event in r['plan']['events']:
            event['sourceRefs'] = [mapping.get(ref, ref) for ref in event['sourceRefs']]
        for unit in r['draft']['units']:
            for shot in unit['shots']:
                shot['sourceRefs'] = [mapping.get(ref, ref) for ref in shot['sourceRefs']]
    add('line-id-fallback', line_ids)
    for key in ('workPackages', 'provisionalUnits'):
        add('reject-half-plan-' + key, lambda b, r, key=key: r['plan'].update({key: []}))
    for i, value in enumerate((True, '2.5', 0, -1, Decimal('0.0000000001'),
                               Decimal('9007199.254740992'), Decimal('1e300'),
                               Decimal('1e-300'))):
        add('reject-duration-' + str(i), lambda b, r, value=value:
            r['draft']['units'][0]['shots'][0].update(duration=value))
    add('reject-unit-tick-overflow', lambda b, r: [
        shot.update(duration=Decimal('9007199.254740991')) for shot in r['draft']['units'][0]['shots']])
    add('max-exact-ticks', lambda b, r: (
        r['draft']['units'][0]['shots'][0].update(duration=Decimal('9007199.254740990')),
        r['draft']['units'][0]['shots'][1].update(duration=Decimal('0.000000001'))))
    add('safe-trailing-zeros-and-exponents', lambda b, r: (
        r['draft']['units'][0]['shots'][0].update(duration=Decimal('1.0000000000')),
        r['draft']['units'][0]['shots'][1].update(duration=Decimal('1e-9'))))
    for i, refs in enumerate(([], ['unknown'], ['PREV:R1'], ['E01:R1', 'E01:R1'], ['E01:R3'])):
        add('reject-source-refs-' + str(i), lambda b, r, refs=refs:
            r['draft']['units'][0]['shots'][0].update(sourceRefs=refs))
    add('reject-context-ref', lambda b, r: r['draft']['units'][0]['shots'][0].update(contextRefs=['unknown']))
    add('reject-duplicate-context-ref', lambda b, r: r['draft']['units'][0]['shots'][0].update(contextRefs=['PREV:R1'] * 2))
    add('reject-duplicate-source-id', lambda b, r:
        b['episodes'][0]['source']['contextRows'][0].update(id='E01:R1'))
    for key in ('emotionArc', 'entryFacts', 'fantasyCandidates'):
        add('reject-missing-' + key, lambda b, r, key=key: r['plan'].pop(key))
    for key in ('emotionContract', 'boundaryBefore'):
        add('reject-missing-event-' + key, lambda b, r, key=key: r['plan']['events'][0].pop(key))
    add('reject-unused-event', lambda b, r: r['plan']['events'].append(
        dict(deepcopy(r['plan']['events'][1]), id='EV3')))
    add('reject-duplicate-event', lambda b, r: r['plan']['events'].append(deepcopy(r['plan']['events'][0])))
    add('reject-unknown-event', lambda b, r: r['draft']['units'][0].update(eventId='unknown'))
    add('reject-prototype-event', lambda b, r: r['plan']['events'][0].update(id='__proto__'))
    add('reject-empty-shots', lambda b, r: r['draft']['units'][0].update(shots=[]))
    add('reject-duplicate-unit', lambda b, r: r['draft']['units'].append(deepcopy(r['draft']['units'][0])))
    add('reject-duplicate-shot', lambda b, r:
        r['draft']['units'][0]['shots'].append(deepcopy(r['draft']['units'][0]['shots'][0])))
    add('reject-conflicting-base-hash', expected_hash='0' * 64)
    add('reject-protected-field', lambda b, r: r.update(source={'replaced': True}))
    add('reject-duplicate-response-key', raw_mutate=lambda b, r: (b, r[:-1] + b',"plan":{}}'))
    payload = {'upstreamCommit': 'f855227c8406b38093d03c777fe24a0b0b4f87dc',
               'sourceHashes': {name: sha256((BASE / 'scripts' / name).read_bytes())
                                for name in ('lite_edit.py', 'context_packet.py')}, 'cases': cases}
    output = HERE / 'series_lite_derive_plan_golden.json'
    output.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'cases': len(cases), 'accepted': sum(c['ok'] for c in cases),
                      'output': str(output)}))


if __name__ == '__main__':
    main()
