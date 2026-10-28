"""Behavior checks for lossless creative assembly; not storyboard quality tests."""
from copy import deepcopy
from decimal import Decimal
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE / 'scripts'))
from context_packet import canonical, parse_json, sha256
from lite_edit import preflight

SCRIPT = BASE / 'scripts/lite_edit.py'


class LiteEditTests(unittest.TestCase):
    def preflight_base(self):
        baseline = json.loads((BASE / 'tests/fixtures/visual-baseline.json').read_text(encoding='utf-8'))
        self.base['generationTextVersion'] = 'voice-delivery-v1'
        self.base['generationBoundaryVersion'] = 'video-prompt-v1'
        self.base['sourceLockHash'] = 'a' * 64
        self.episode['settings']['visualBaseline'] = baseline
        self.episode['settings']['visualStyle'] = baseline['visualStyle']
        self.episode['settings']['prohibitions'] = '保留已采用项目禁止项。'
        self.episode['settings']['maxUnitSeconds'] = 30
        self.episode['settings']['fantasyMode'] = 'off'
        self.episode['plan'] = {}
        self.episode['draft'] = {}
        self.save('preflight.json', self.base)

    def test_preflight_allows_empty_creative_fields_and_never_mutates_baseline(self):
        self.preflight_base()
        raw = (self.job / 'preflight.json').read_bytes()
        result = subprocess.run([sys.executable, str(SCRIPT), 'preflight', '--workdir', str(self.job),
                                 '--base', 'preflight.json'], capture_output=True, text=True, encoding='utf-8')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)['validation'], 'frozen-setup-fields-only')
        self.assertEqual(raw, (self.job / 'preflight.json').read_bytes())

    def test_preflight_requires_generation_boundary_in_new_baseline(self):
        for value in (None, 'unknown'):
            self.preflight_base()
            if value is None:
                self.base.pop('generationBoundaryVersion')
            else:
                self.base['generationBoundaryVersion'] = value
            self.save('preflight.json', self.base)
            with self.assertRaisesRegex(ValueError, 'generationBoundaryVersion'):
                preflight(self.job, 'preflight.json')

    def test_preflight_rejects_wrong_settings_before_creative_generation(self):
        for key, value in [('prohibitions', ['禁止项']), ('visualStyle', '缩略画风'),
                           ('maxUnitSeconds', 31), ('fantasyMode', 'unknown')]:
            with self.subTest(key=key):
                self.preflight_base()
                self.episode['settings'][key] = value
                self.save('preflight.json', self.base)
                with self.assertRaisesRegex(ValueError, key):
                    preflight(self.job, 'preflight.json')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='lite-edit-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.job = self.root / 'job'
        self.job.mkdir()
        self.base = parse_json((BASE / 'tests/fixtures/lingmo-v7-bundle.json').read_bytes(), 'fixture')
        self.base['unknownMetadata'] = {'retained': ['原值', None, False, 9007199254740993]}
        self.episode = self.base['episodes'][0]
        self.episode['settings']['precision'] = Decimal('0.123456789012345678901234567890')
        self.episode['draft']['units'][0]['shots'][0]['duration'] = Decimal('2.123456789012345678901')
        self.save('base.json', self.base)
        self.raw = (self.job / 'base.json').read_bytes()
        self.hash = sha256(self.raw)
        self.creative = {k: deepcopy(self.episode[k]) for k in ('plan', 'selection', 'draft')}

    def save(self, name, value):
        (self.job / name).write_bytes(canonical(value))

    def cli(self, mode, *, output='out.json', base='base.json', source=None, ok=True, digest=None):
        if source is None:
            source = 'response.json' if mode == 'compose' else 'patch.json'
        args = [sys.executable, str(SCRIPT), mode, '--workdir', str(self.job),
                '--base', base, '--output', output]
        args += (['--response', source, '--expected-base-sha256', digest or self.hash]
                 if mode == 'compose' else ['--patch', source])
        result = subprocess.run(args, capture_output=True, text=True, encoding='utf-8', timeout=20)
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        if not ok:
            self.assertEqual(result.stdout, '')
            self.assertIn('lite_edit:', result.stderr)
            return result.stderr
        summary = json.loads(result.stdout)
        self.assertEqual(summary['validation'], 'not-run')
        self.assertNotIn('sourceRows', result.stdout)
        raw = (self.job / output).read_bytes()
        self.assertEqual(summary['sha256'], sha256(raw))
        self.assertEqual(summary['bytes'], len(raw))
        return parse_json(raw, output), summary

    def operation(self, path='/episodes/0/draft/units/0/title', value='修订标题'):
        return {'op': 'set', 'path': path, 'expected': self.episode['draft']['units'][0]['title'],
                'value': value}

    def patch_request(self, operations, **extra):
        request = {'baseSha256': self.hash, 'operations': operations, **extra}
        self.save('patch.json', request)
        return request

    def test_compose_preserves_all_noncreative_values_and_source_bytes_in_values(self):
        self.creative['draft']['units'][0]['title'] = '新的完整表演稿'
        self.save('response.json', self.creative)
        result, summary = self.cli('compose')
        expected = deepcopy(self.base)
        expected['episodes'][0].update(self.creative)
        self.assertEqual(canonical(result), canonical(expected))
        self.assertEqual((self.job / 'base.json').read_bytes(), self.raw)
        self.assertTrue(summary['changed'])

    def test_compose_refuses_noncreative_and_missing_response_keys(self):
        for key in ('source', 'settings', 'scenes', 'assets', 'scope', 'expectedBaseSha256'):
            with self.subTest(key=key):
                self.save('response.json', {**self.creative, key: {}})
                self.cli('compose', ok=False)
                self.assertFalse((self.job / 'out.json').exists())
        self.save('response.json', {'draft': self.creative['draft']})
        self.cli('compose', ok=False)

    def test_does_not_invent_missing_base_or_frozen_source_fields(self):
        self.save('response.json', self.creative)
        self.cli('compose', base='absent.json', ok=False)
        for key in ('source', 'settings', 'scenes'):
            base = deepcopy(self.base)
            del base['episodes'][0][key]
            self.save('incomplete.json', base)
            self.cli('compose', base='incomplete.json', digest=sha256(canonical(base)), ok=False)
        self.assertFalse((self.job / 'out.json').exists())

    def test_patch_changes_only_requested_value_and_never_autocorrects_voice(self):
        text_path = '/episodes/0/draft/units/0/shots/0/dialogueParts/0/text'
        old = self.episode['draft']['units'][0]['shots'][0]['dialogueParts'][0]['text']
        operation = {'op': 'set', 'path': text_path, 'expected': old, 'value': '故意不吻合原声，等后续核查'}
        self.patch_request([operation])
        result, _ = self.cli('patch')
        expected = deepcopy(self.base)
        expected['episodes'][0]['draft']['units'][0]['shots'][0]['dialogueParts'][0]['text'] = operation['value']
        self.assertEqual(canonical(result), canonical(expected))
        self.assertEqual(result['episodes'][0]['source'], self.episode['source'])

    def test_rejects_frozen_fields_other_episodes_and_root(self):
        for path in ('', '/episodes', '/episodes/1/draft', '/episodes/00/draft',
                     '/episodes/0/source', '/episodes/0/settings', '/episodes/0/scenes',
                     '/episodes/0/number', '/assets', '/scope'):
            with self.subTest(path=path):
                self.patch_request([self.operation(path)])
                self.cli('patch', ok=False)
                self.assertFalse((self.job / 'out.json').exists())

    def test_expected_mismatch_rejects_entire_patch(self):
        good = self.operation()
        bad = {'op': 'set', 'path': '/episodes/0/plan', 'expected': {'wrong': True}, 'value': {}}
        self.patch_request([good, bad])
        self.assertIn('expected value', self.cli('patch', ok=False))
        self.assertFalse((self.job / 'out.json').exists())
        self.assertEqual((self.job / 'base.json').read_bytes(), self.raw)

    def test_bool_does_not_match_number_in_expected(self):
        self.episode['draft']['flag'] = True
        self.save('base.json', self.base)
        self.hash = sha256(canonical(self.base))
        self.patch_request([{'op': 'set', 'path': '/episodes/0/draft/flag', 'expected': 1, 'value': False}])
        self.assertIn('expected value', self.cli('patch', ok=False))

    def test_stale_hash_rejected_for_both_commands(self):
        self.save('response.json', self.creative)
        self.patch_request([self.operation()])
        (self.job / 'base.json').write_bytes(self.raw + b'\n')
        for mode in ('compose', 'patch'):
            self.assertIn('SHA-256 mismatch', self.cli(mode, ok=False))

    def test_patch_rejects_unknown_keys_and_unsupported_operations(self):
        self.patch_request([], unknown=True)
        self.cli('patch', ok=False)
        for change in ({'unknown': 1}, {'op': 'remove'}, {'path': 7}):
            self.patch_request([{**self.operation(), **change}])
            self.cli('patch', ok=False)

    def test_overlap_and_duplicate_paths_rejected_regardless_of_order(self):
        parent = {'op': 'set', 'path': '/episodes/0/draft',
                  'expected': self.episode['draft'], 'value': self.episode['draft']}
        for operations in ([parent, self.operation()], [self.operation(), parent],
                           [self.operation(), self.operation()]):
            self.patch_request(operations)
            self.assertIn('overlapping', self.cli('patch', ok=False))

    def test_bad_escapes_missing_keys_and_array_bounds_rejected(self):
        for suffix in ('/units/-', '/units/999', '/units/00', '/units/0/title/x',
                       '/missing', '/bad~2key'):
            self.patch_request([self.operation('/episodes/0/draft' + suffix)])
            self.cli('patch', ok=False)

    def test_replaces_whole_units_array_without_inventing_source_validation(self):
        units = self.episode['draft']['units']
        self.patch_request([{'op': 'set', 'path': '/episodes/0/draft/units',
                             'expected': units, 'value': units[:1]}])
        result, summary = self.cli('patch')
        self.assertEqual(result['episodes'][0]['draft']['units'], units[:1])
        self.assertEqual(result['episodes'][0]['source'], self.episode['source'])
        self.assertEqual(summary['validation'], 'not-run')

    def test_decimal_precision_and_escaped_pointer_are_lossless(self):
        old = Decimal('1.123456789012345678901234567890')
        new = Decimal('1.123456789012345678901234567891')
        self.episode['draft']['a/b'] = {'~number': old}
        self.save('base.json', self.base)
        self.hash = sha256(canonical(self.base))
        self.patch_request([{'op': 'set', 'path': '/episodes/0/draft/a~1b/~0number',
                             'expected': old, 'value': new}])
        result, _ = self.cli('patch')
        self.assertEqual(result['episodes'][0]['draft']['a/b']['~number'], new)
        self.assertEqual(result['episodes'][0]['settings']['precision'], self.episode['settings']['precision'])
        self.assertEqual(result['episodes'][0]['draft']['units'], self.episode['draft']['units'])

    def test_no_change_copies_exact_base_bytes_and_reports_false(self):
        (self.job / 'base.json').write_bytes(self.raw + b'\n\n')
        self.hash = sha256(self.raw + b'\n\n')
        self.save('response.json', self.creative)
        _, summary = self.cli('compose', output='composed.json')
        self.assertFalse(summary['changed'])
        self.assertEqual(summary['sha256'], self.hash)
        self.patch_request([])
        _, summary = self.cli('patch')
        self.assertFalse(summary['changed'])
        self.assertEqual((self.job / 'out.json').read_bytes(), self.raw + b'\n\n')

    def test_existing_output_and_base_cannot_be_overwritten(self):
        self.save('response.json', self.creative)
        (self.job / 'out.json').write_bytes(b'protected output')
        self.cli('compose', ok=False)
        self.assertEqual((self.job / 'out.json').read_bytes(), b'protected output')
        self.patch_request([])
        self.cli('patch', output='base.json', ok=False)
        self.assertEqual((self.job / 'base.json').read_bytes(), self.raw)

    def test_all_input_and_output_paths_stay_inside_job(self):
        (self.root / 'outside.json').write_bytes(self.raw)
        self.save('response.json', self.creative)
        for options in ({'base': '../outside.json'}, {'base': str(self.root / 'outside.json')},
                        {'source': '../outside.json'}, {'output': '../new.json'},
                        {'output': str(self.root / 'new.json')}):
            self.cli('compose', ok=False, **options)
        self.patch_request([])
        self.cli('patch', source='../outside.json', ok=False)
        self.assertFalse((self.root / 'new.json').exists())

    def test_duplicate_keys_and_nonfinite_values_rejected(self):
        for raw in (b'{"plan":{},"plan":{},"selection":{},"draft":{}}',
                    b'{"plan":{},"selection":{},"draft":{"bad":NaN}}'):
            (self.job / 'response.json').write_bytes(raw)
            self.cli('compose', ok=False)

    def test_multiple_episode_base_rejected(self):
        self.base['episodes'].append(deepcopy(self.episode))
        self.save('base.json', self.base)
        self.hash = sha256(canonical(self.base))
        self.save('response.json', self.creative)
        self.assertIn('exactly one episode', self.cli('compose', ok=False))


if __name__ == '__main__':
    unittest.main()
