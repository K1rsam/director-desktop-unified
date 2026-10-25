"""Read-only editing signals: episode boundaries, coordinates and exact source data."""
import hashlib
import json
from decimal import Decimal
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'inspect_editing.py'


def draft(count=3):
    return {'units': [{'id': 'U1', 'lighting': '单元光', 'shots': [
        {'id': f'S{i + 1}', 'duration': 2.75, 'lighting': ' 同一光向。\n',
         'transition': '动作接切。',
         'dialogueParts': [{'lineId': 'L1', 'text': f'原文{i}，'}]}
        for i in range(count)]}]}


class InspectEditingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source, self.output = self.root / 'input.json', self.root / 'report.json'

    def run_cli(self, value, raw=None, ok=True):
        data = raw if raw is not None else json.dumps(value, ensure_ascii=False).encode()
        self.source.write_bytes(data)
        result = subprocess.run([sys.executable, str(SCRIPT), '--input', str(self.source),
                                 '--output', str(self.output)], capture_output=True, text=True)
        self.assertEqual(self.source.read_bytes(), data)
        if not ok:
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, '')
            self.assertIn('inspect_editing:', result.stderr)
            return result
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(self.output.read_text(), parse_float=Decimal)
        self.assertEqual(report['input_sha256'], hashlib.sha256(data).hexdigest())
        return report, json.loads(result.stdout)

    def test_repeated_fields_keep_exact_text_and_coordinates(self):
        value = draft()
        third = value['units'][0]['shots'].pop()
        value['units'].append({'id': 'U2', 'shots': [third]})
        report, summary = self.run_cli(value)
        groups = report['episodes'][0]['repeated_text']
        self.assertEqual({g['field'] for g in groups}, {'lighting', 'transition'})
        light = next(g for g in groups if g['field'] == 'lighting')
        self.assertEqual(light['text'], ' 同一光向。\n')
        self.assertEqual(light['count'], 3)
        self.assertEqual([(p['unit'], p['shot']) for p in light['occurrences']],
                         [('U1', 'S1'), ('U1', 'S2'), ('U2', 'S3')])
        self.assertEqual(summary['repeated_text_groups'], 2)
        self.assertNotIn('同一光向', json.dumps(summary, ensure_ascii=False))

    def test_timeline_unions_overlap_and_measures_across_unit_boundary(self):
        value = {'units': [
            {'id': 'U1', 'shots': [{'id': 'S1', 'duration': 4, 'dialogueParts': [
                {'lineId': 'L1', 'text': '甲', 'voiceStart': 0.5, 'voiceEnd': 2.5},
                {'lineId': 'L2', 'text': '乙', 'voiceStart': 2, 'voiceEnd': 3}]}]},
            {'id': 'U2', 'shots': [{'id': 'S2', 'duration': 2, 'dialogueParts': []},
                {'id': 'S3', 'duration': 3, 'dialogueParts': [
                {'lineId': 'L3', 'text': '丙', 'voiceStart': 1, 'voiceEnd': 2}]}]}]}
        report, _ = self.run_cli(value)
        time = report['episodes'][0]['timeline']
        self.assertEqual(time['total_seconds'], 9)
        self.assertEqual(time['voice_seconds'], Decimal('3.5'))
        self.assertEqual(time['outside_voice_seconds'], Decimal('5.5'))
        gap = time['longest_outside_voice_spans'][0]
        self.assertEqual((gap['start'], gap['end'], gap['seconds']), (3, 7, 4))
        self.assertEqual(gap['previous']['unit'], 'U1')
        self.assertEqual(gap['next']['unit'], 'U2')
        self.assertEqual(time['adjacent_line_intervals'][0]['seconds'], Decimal('-0.5'))

    def test_missing_windows_do_not_claim_silence_or_complete_timing(self):
        report, _ = self.run_cli(draft())
        time = report['episodes'][0]['timeline']
        self.assertIsNone(time['voice_seconds'])
        self.assertIsNone(time['outside_voice_seconds'])
        self.assertEqual(len(time['missing_voice_windows']), 3)
        self.assertEqual(time['longest_outside_voice_spans'], [])

    def test_empty_speech_reports_action_span_without_quality_judgment(self):
        value = {'units': [{'id': 'U1', 'shots': [
            {'id': 'S1', 'duration': 5, 'dialogueParts': []}]}]}
        report, _ = self.run_cli(value)
        time = report['episodes'][0]['timeline']
        self.assertEqual(time['voice_seconds'], 0)
        self.assertEqual(time['outside_voice_seconds'], 5)
        self.assertEqual(time['longest_outside_voice_spans'][0]['seconds'], 5)
        self.assertEqual(time['adjacent_line_intervals'], [])

    def test_invalid_voice_window_is_not_clamped(self):
        for start, end in [(-1, 1), (2, 1), (0, 4), (True, 1)]:
            with self.subTest(start=start, end=end):
                value = draft(1)
                value['units'][0]['shots'][0]['dialogueParts'][0].update(
                    voiceStart=start, voiceEnd=end)
                self.run_cli(value, ok=False)
                self.assertFalse(self.output.exists())

    def test_same_line_and_text_in_two_episodes_do_not_merge(self):
        report, _ = self.run_cli({'episodes': [{'number': n, 'draft': draft(2)} for n in (1, 2)]})
        self.assertEqual([e['number'] for e in report['episodes']], [1, 2])
        for episode in report['episodes']:
            self.assertEqual(episode['repeated_text'], [])
            self.assertEqual(episode['dialogue_spans'], [])

    def test_special_episode_integer_numbers_remain_valid(self):
        report, _ = self.run_cli({'episodes': [{'number': n, 'draft': draft(2)} for n in (0, -7)]})
        self.assertEqual([e['number'] for e in report['episodes']], [0, -7])

    def test_episode_number_rejects_bool_and_duplicate(self):
        for numbers in ((True,), (0, 0)):
            with self.subTest(numbers=numbers):
                self.run_cli({'episodes': [{'number': n, 'draft': draft(2)} for n in numbers]}, ok=False)
                self.assertFalse(self.output.exists())

    def test_unit_headers_are_excluded_and_whitespace_is_not_normalized(self):
        value = draft(4)
        value['units'][0]['lighting'] = ' 同一光向。\n'
        value['units'][0]['shots'][2]['lighting'] = '同一光向。'
        value['units'][0]['shots'][3]['lighting'] = '同一光向。'
        report, _ = self.run_cli(value)
        self.assertFalse(any(g['field'] == 'lighting'
                             for g in report['episodes'][0]['repeated_text']))

    def test_multiple_parts_in_one_shot_count_as_one_shot(self):
        value = draft(2)
        value['units'][0]['shots'][0]['dialogueParts'] *= 3
        report, _ = self.run_cli({'number': 7, 'draft': value})
        self.assertEqual(report['episodes'][0]['dialogue_spans'], [])

    def test_three_shot_line_preserves_part_order_text_and_decimal(self):
        value = draft()
        value['units'][0]['shots'][0]['dialogueParts'].append({'lineId': 'L1', 'text': ' 续句\n'})
        raw = json.dumps(value, ensure_ascii=False).replace('2.75', '2.750000000000000000001').encode()
        report, _ = self.run_cli(None, raw=raw)
        group = report['episodes'][0]['dialogue_spans'][0]
        self.assertEqual(group['lineId'], 'L1')
        self.assertEqual(group['shot_count'], 3)
        self.assertEqual(group['text'], '原文0， 续句\n原文1，原文2，')
        self.assertEqual([p['shot'] for p in group['shots']], ['S1', 'S2', 'S3'])
        self.assertEqual(group['shots'][0]['duration'], Decimal('2.750000000000000000001'))

    def test_report_is_explicitly_advisory_without_pass_fields(self):
        report, _ = self.run_cli(draft())
        self.assertTrue(report['advisory_only'])
        self.assertIn('内容复核', report['limitations'])
        def inspect(item):
            if isinstance(item, dict):
                self.assertFalse(any('pass' in key.lower() for key in item))
                for child in item.values():
                    inspect(child)
            elif isinstance(item, list):
                for child in item:
                    inspect(child)
        inspect(report)

    def test_cut_pairs_expose_actual_shots_and_boundary_without_inventing_a_match(self):
        value = {'units': [
            {'id': 'U1', 'eventId': 'EV1', 'scene': '门内',
             'exitState': '钥匙仍在向门外飞，右手尚未收回。', 'shots': [
                 {'id': 'S1', 'duration': 1.25, 'camera': '侧面近景',
                  'picture': '钥匙刚越过右侧门框，仍向门外飞。\n手停在松开后的相位。',
                  'transition': '动作顺接。', 'sound': '风声', 'sourceRefs': ['R1'],
                  'dialogueParts': [{'lineId': 'L1', 'text': '拿稳，',
                                     'voiceStart': 0, 'voiceEnd': 1}]}]},
            {'id': 'U2', 'eventId': 'EV1', 'scene': '门内',
             'boundaryBefore': {'kind': 'capacity', 'reason': '同一次飞行继续'},
             'initialState': '钥匙仍在飞。', 'shots': [
                 {'id': 'S1', 'duration': 1, 'picture': '杜衡已经站稳，钥匙在掌心。',
                  'transition': '切向阿澜。', 'dialogueParts': []},
                 {'id': 'S2', 'duration': 1, 'picture': '阿澜仍抱匣。',
                  'transition': '结束。', 'dialogueParts': []}]}]}
        report, summary = self.run_cli(value)
        self.assertIn('cut_pairs', report['episodes'][0])
        pairs = report['episodes'][0]['cut_pairs']
        self.assertEqual(len(pairs), 2)
        edge = pairs[0]
        self.assertTrue(edge['crosses_unit'])
        self.assertEqual((edge['outgoing']['unit'], edge['outgoing']['shot']), ('U1', 'S1'))
        self.assertEqual((edge['incoming']['unit'], edge['incoming']['shot']), ('U2', 'S1'))
        self.assertEqual(edge['outgoing']['picture'], value['units'][0]['shots'][0]['picture'])
        self.assertEqual(edge['incoming']['picture'], '杜衡已经站稳，钥匙在掌心。')
        self.assertEqual(edge['outgoing']['dialogueParts'], value['units'][0]['shots'][0]['dialogueParts'])
        self.assertEqual(edge['unit_boundary']['outgoing']['exitState'], value['units'][0]['exitState'])
        self.assertEqual(edge['unit_boundary']['incoming']['boundaryBefore'], value['units'][1]['boundaryBefore'])
        self.assertFalse(pairs[1]['crosses_unit'])
        self.assertIsNone(pairs[1]['unit_boundary'])
        self.assertNotIn('钥匙', json.dumps(summary, ensure_ascii=False))
        self.assertNotIn('match', edge)

    def test_cut_pairs_do_not_bridge_episodes_or_empty_units_and_keep_real_scene_change(self):
        def unit(uid, scene, kind):
            return {'id': uid, 'scene': scene, 'boundaryBefore': {'kind': kind, 'reason': '源文'},
                    'shots': [{'id': 'S1', 'duration': 1, 'picture': scene}]}
        units = [unit('U1', '夜 库房', 'episode_start'),
                 unit('U2', '翌日 河岸', 'time_space_jump'),
                 {'id': 'U3', 'shots': []}, unit('U4', '河岸', 'continuous')]
        report, _ = self.run_cli({'episodes': [{'number': n, 'draft': {'units': units}} for n in (1, 2)]})
        for episode in report['episodes']:
            self.assertIn('cut_pairs', episode)
            self.assertEqual(len(episode['cut_pairs']), 1)
            boundary = episode['cut_pairs'][0]['unit_boundary']
            self.assertEqual(boundary['incoming']['scene'], '翌日 河岸')
            self.assertEqual(boundary['incoming']['boundaryBefore']['kind'], 'time_space_jump')
            self.assertNotIn('initialState', boundary['incoming'])

    def test_merged_internal_cut_keeps_each_shots_original_segment_state(self):
        value = draft(2)
        value['units'][0]['segments'] = [
            {'id': 'part-a', 'shotIds': ['S1'], 'exitState': '铜环仍在滑动。'},
            {'id': 'part-b', 'shotIds': ['S2'], 'initialState': '乙还未接住。',
             'exitState': '铜环在左手。'}]
        report, _ = self.run_cli(value)
        pair = report['episodes'][0]['cut_pairs'][0]
        self.assertIn('segment_headers', pair['outgoing'])
        self.assertEqual(pair['outgoing']['segment_headers'], value['units'][0]['segments'][:1])
        self.assertEqual(pair['incoming']['segment_headers'], value['units'][0]['segments'][1:])
        self.assertFalse(pair['crosses_unit'])

    def test_invalid_structure_is_explicit_and_creates_no_output(self):
        cases = [[], {}, {'units': {}}, {'episodes': [None]},
                 {'units': [{'id': 'U1', 'shots': [{}]}]},
                 {'units': [{'id': 'U1', 'shots': 'bad'}]},
                 {'units': [{'id': 'U1', 'shots': [{'id': 'S1', 'duration': 1,
                            'dialogueParts': [{'text': 'missing lineId'}]}]}]}]
        for value in cases:
            with self.subTest(value=value):
                result = self.run_cli(value, ok=False)
                self.assertIn('invalid input', result.stderr)
                self.assertFalse(self.output.exists())

    def test_output_refuses_overwrite_and_keeps_existing_bytes(self):
        self.output.write_bytes(b'KEEP')
        result = self.run_cli(draft(), ok=False)
        self.assertIn('exists', result.stderr)
        self.assertEqual(self.output.read_bytes(), b'KEEP')

    def test_large_groups_keep_every_coordinate_only_in_report(self):
        report, summary = self.run_cli(draft(1000))
        self.assertEqual(len(report['episodes'][0]['repeated_text'][0]['occurrences']), 1000)
        self.assertEqual(len(report['episodes'][0]['dialogue_spans'][0]['shots']), 1000)
        self.assertLess(len(json.dumps(summary)), 600)


if __name__ == '__main__':
    unittest.main()
