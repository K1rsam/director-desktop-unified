"""Retrospective structure derivation: source ownership, exact time, no authorship."""
from copy import deepcopy
from decimal import Decimal, localcontext
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE / 'scripts'))
from context_packet import PacketError, canonical, parse_json, sha256
from lite_edit import compose


class DerivePlanTests(unittest.TestCase):
    def setUp(self):
        self.base = parse_json((BASE / 'tests/fixtures/lingmo-v7-bundle.json').read_bytes(), 'fixture')
        episode = self.base['episodes'][0]
        episode['source']['contextRows'] = [
            {'id': 'PREV:R1', 'text': '△此前门厅闭门。'},
            {'id': 'PREV:R2', 'text': '△此前钥匙已交给值守者。'}]
        self.response = {key: deepcopy(episode[key]) for key in ('plan', 'selection', 'draft')}
        plan, draft = self.response['plan'], self.response['draft']
        plan.update({'emotionArc': '从戒备到确认门锁正常', 'entryFacts': [
            {'id': 'F1', 'subject': '门', 'statement': '门厅闭门',
             'sourceRefs': ['PREV:R1'], 'evidenceMode': 'source_rows'},
            {'id': 'F2', 'subject': '钥匙', 'statement': '值守者持有钥匙',
             'sourceRefs': ['PREV:R2'], 'evidenceMode': 'source_rows'}], 'events': []})
        for index, unit in enumerate(draft['units']):
            event_id = 'EV' + str(index + 1)
            unit['eventId'] = event_id
            unit['boundaryBefore'] = {'kind': 'episode_start' if index == 0 else 'new_event',
                                      'reason': '源文依序进入所标场景'}
            plan['events'].append({
                'id': event_id, 'sceneKey': unit['sceneId'],
                'boundaryBefore': deepcopy(unit['boundaryBefore']),
                'sourceRefs': list(dict.fromkeys(ref for shot in unit['shots'] for ref in shot['sourceRefs'])),
                'emotionContract': {'subject': '门锁状态', 'contextTone': '戒备',
                                    'audienceEffect': '关注', 'change': '得到确认',
                                    'landing': '保持警戒', 'retain': ['门尚未打开']}})
        draft['units'][0]['shots'][0]['contextRefs'] = ['PREV:R1']

    def derived(self, response=None):
        return compose(self.base, response or self.response, derive_plan=True)['episodes'][0]

    def test_indices_copy_event_contract_and_use_only_actual_context(self):
        before = canonical(self.response)
        episode = self.derived()
        packages = episode['plan']['workPackages']
        self.assertEqual(len(packages), len(self.response['plan']['events']))
        for package, event in zip(packages, self.response['plan']['events']):
            self.assertEqual(package['id'], event['id'])
            self.assertEqual(package['eventId'], event['id'])
            self.assertEqual(package['sourceRefs'], event['sourceRefs'])
            self.assertEqual(package['emotionContract'], event['emotionContract'])
            self.assertEqual(package['dependsOn'], [])
            self.assertEqual(package['scenarioKeys'], [])
        self.assertEqual(packages[0]['contextRefs'], ['PREV:R1'])
        self.assertEqual(packages[0]['entryFactIds'], ['F1'])
        self.assertEqual(packages[1]['contextRefs'], [])
        self.assertEqual(packages[1]['entryFactIds'], [])
        self.assertEqual(canonical(episode['draft']), canonical(self.response['draft']))
        self.assertEqual(canonical(episode['selection']), canonical(self.response['selection']))
        self.assertEqual(canonical(episode['source']), canonical(self.base['episodes'][0]['source']))
        self.assertEqual(canonical(self.response), before)

    def test_source_union_in_frozen_order_and_nanosecond_sum_without_rounding(self):
        shots = self.response['draft']['units'][0]['shots']
        refs = self.response['plan']['events'][0]['sourceRefs']
        shots[0]['sourceRefs'] = list(reversed(refs))
        shots[1]['sourceRefs'] = refs[:1]
        shots[0]['duration'] = Decimal('0.100000001')
        shots[1]['duration'] = Decimal('0.200000002')
        # Arithmetic must remain exact even if the caller changes Decimal context.
        with localcontext() as context:
            context.prec = 3
            index = self.derived()['plan']['provisionalUnits'][0]
        self.assertEqual(index['sourceRefs'], refs)
        self.assertEqual(index['secondsRange'], [Decimal('0.300000003')] * 2)
        self.assertEqual(index['id'], self.response['draft']['units'][0]['id'])
        self.assertEqual(index['packageId'], 'EV1')

    def test_old_default_and_explicit_plan_are_unchanged(self):
        self.assertEqual(compose(self.base, self.response)['episodes'][0]['plan'], self.response['plan'])
        response = deepcopy(self.response)
        response['plan']['workPackages'] = [{'opaque': 'authored'}]
        response['plan']['provisionalUnits'] = [{'opaque': 'authored'}]
        self.assertEqual(self.derived(response)['plan'], response['plan'])

    def test_derived_seconds_do_not_add_artificial_precision_to_patch_expected(self):
        shots = self.response['draft']['units'][0]['shots']
        shots[0]['duration'], shots[1]['duration'] = Decimal('6.1'), Decimal('12.5')
        seconds = self.derived()['plan']['provisionalUnits'][0]['secondsRange']
        self.assertEqual(canonical(seconds), b'[18.6,18.6]')

    def test_refuses_mixed_missing_plan_fields(self):
        for key in ('workPackages', 'provisionalUnits'):
            response = deepcopy(self.response)
            response['plan'][key] = []
            with self.assertRaisesRegex(PacketError, 'mixed missing'):
                self.derived(response)

    def test_refuses_unsafe_durations(self):
        for value in (True, '2.5', 0, -1, Decimal('NaN'), Decimal('Infinity'),
                      Decimal('0.0000000001'), Decimal('9007199.254740992')):
            with self.subTest(value=str(value)):
                response = deepcopy(self.response)
                response['draft']['units'][0]['shots'][0]['duration'] = value
                with self.assertRaisesRegex(PacketError, 'duration'):
                    self.derived(response)

    def test_refuses_unknown_empty_duplicate_or_cross_event_references(self):
        for refs in ([], ['unknown'], ['PREV:R1'], ['E01:R1', 'E01:R1'], ['E01:R3']):
            with self.subTest(refs=refs):
                response = deepcopy(self.response)
                response['draft']['units'][0]['shots'][0]['sourceRefs'] = refs
                with self.assertRaises(PacketError):
                    self.derived(response)
        response = deepcopy(self.response)
        response['draft']['units'][0]['shots'][0]['contextRefs'] = ['unknown']
        with self.assertRaisesRegex(PacketError, 'contextRefs'):
            self.derived(response)

    def test_refuses_missing_or_duplicate_event_and_missing_author_work(self):
        mutations = [
            lambda r: r['plan']['events'].append(deepcopy(r['plan']['events'][0])),
            lambda r: r['draft']['units'][0].pop('eventId'),
            lambda r: r['draft']['units'][0].update(eventId='unknown'),
            lambda r: r['plan'].pop('emotionArc'),
            lambda r: r['plan'].pop('entryFacts'),
            lambda r: r['plan'].pop('fantasyCandidates'),
            lambda r: r['plan']['events'][0].pop('emotionContract'),
            lambda r: r['plan']['events'][0].pop('boundaryBefore'),
            lambda r: r['draft']['units'][0].update(shots=[]),
        ]
        for mutation in mutations:
            response = deepcopy(self.response)
            mutation(response)
            with self.assertRaises(PacketError):
                self.derived(response)

    def test_cli_marks_retrospective_basis_and_normal_checks_remain_authoritative(self):
        with tempfile.TemporaryDirectory(prefix='derive-plan-') as directory:
            root = Path(directory)
            (root / 'base.json').write_bytes(canonical(self.base))
            (root / 'response.json').write_bytes(canonical(self.response))
            args = [sys.executable, str(BASE / 'scripts/lite_edit.py'), 'compose',
                    '--workdir', directory, '--base', 'base.json', '--response', 'response.json',
                    '--output', 'out.json', '--expected-base-sha256', sha256(canonical(self.base)),
                    '--derive-plan']
            result = subprocess.run(args, capture_output=True, text=True, encoding='utf-8', timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            summary = json.loads(result.stdout)
            self.assertTrue(summary['derivedBookkeeping'])
            self.assertEqual(summary['validation'], 'not-run')
            self.assertIn('not prior capacity planning', summary['bookkeepingBasis'])
            storyboard = str(BASE / 'scripts/storyboard.mjs')
            for command in ('check', 'export', 'verify'):
                args = ['node', storyboard, command, '--input', str(root / 'out.json')]
                if command != 'check':
                    args += ['--output', str(root / 'out.md')]
                result = subprocess.run(args, capture_output=True, text=True, encoding='utf-8', timeout=20)
                self.assertEqual(result.returncode, 0, result.stderr)
            bad = parse_json((root / 'out.json').read_bytes(), 'output')
            bad['episodes'][0]['draft']['units'][0]['shots'][0]['dialogueParts'][0]['text'] = '错误台词不应被修复'
            # Assembly retains this authored defect; existing quality checks reject it.
            response = {key: bad['episodes'][0][key] for key in ('plan', 'selection', 'draft')}
            reassembled = compose(self.base, response, derive_plan=True)
            self.assertEqual(reassembled['episodes'][0]['draft'], response['draft'])
            (root / 'bad.json').write_bytes(canonical(reassembled))
            result = subprocess.run(['node', storyboard, 'check', '--input', str(root / 'bad.json')],
                                    capture_output=True, text=True, encoding='utf-8', timeout=20)
            self.assertNotEqual(result.returncode, 0)


if __name__ == '__main__':
    unittest.main()
