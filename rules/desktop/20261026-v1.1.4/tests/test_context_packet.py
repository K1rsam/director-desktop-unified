"""Behavior checks for bounded, lossless worker input packets (stdlib only)."""
import hashlib
import json
from decimal import Decimal
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'context_packet.py'


def digest(data):
    return hashlib.sha256(data).hexdigest()


class ContextPacketTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.job = self.root / 'job'
        self.job.mkdir()
        self.original = '第1集\r\n甲：“history、chat、transcript 都是对白。”\r\n末尾空格  \n'
        self.request = {'version': 'series-context-request-v1', 'episode': 1,
                        'role': 'author', 'inputs': []}
        for kind, value in [('source', self.original), ('direction', '已确认方向'),
                            ('state', '{"adopted":false}'), ('assets', '人物甲'),
                            ('entry', 'PRELIMINARY：前集尚未完成，不声称实际尾态')]:
            self.add_input(kind, kind + '.txt', value)

    def add_input(self, kind, path, value, pointer=None):
        raw = value.encode('utf-8') if isinstance(value, str) else value
        target = self.job / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(raw)
        item = {'kind': kind, 'path': path, 'sha256': digest(raw)}
        if pointer is not None:
            item['pointer'] = pointer
        self.request['inputs'].append(item)
        return item

    def save_request(self):
        (self.job / 'request.json').write_text(json.dumps(self.request, ensure_ascii=False), encoding='utf-8')

    def cli(self, *args):
        return subprocess.run([sys.executable, '-X', 'utf8', str(SCRIPT), *map(str, args)],
                              capture_output=True, text=True, encoding='utf-8')

    def build(self, output='packets/one.json', max_bytes=None, ok=True):
        self.save_request()
        args = ['build', '--workdir', self.job, '--manifest', 'request.json', '--output', output]
        if max_bytes is not None:
            args += ['--max-bytes', max_bytes]
        result = self.cli(*args)
        if ok:
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads((self.job / output).read_text(encoding='utf-8')), json.loads(result.stdout)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, '')
        self.assertIn('context_packet:', result.stderr)
        return result.stderr

    def verify(self, ok=True, packet='packets/one.json'):
        result = self.cli('verify', '--workdir', self.job, '--packet', packet)
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        if not ok:
            self.assertEqual(result.stdout, '')
        return result

    def test_preserves_source_bytes_and_preliminary_entry_without_content_filtering(self):
        packet, summary = self.build()
        self.assertEqual(packet['payload'][0]['value'].encode('utf-8'), self.original.encode('utf-8'))
        self.assertIn('PRELIMINARY', packet['payload'][4]['value'])
        self.assertEqual(set(summary), {'path', 'bytes', 'sha256', 'inputs'})
        self.assertEqual(summary['inputs'], 5)
        raw = (self.job / 'packets/one.json').read_bytes()
        self.assertEqual(summary['bytes'], len(raw))
        self.assertEqual(summary['sha256'], digest(raw))
        self.assertNotIn('第1集', json.dumps(summary, ensure_ascii=False))
        self.verify()

    def test_large_unrelated_library_only_includes_selected_exact_subtree(self):
        data = {'episodes': [{'dialogue': self.original, 'timing': 1.25}],
                'unrelated': '无关' * 200000}
        self.request['inputs'] = [x for x in self.request['inputs'] if x['kind'] != 'source']
        self.add_input('source', 'all.json', json.dumps(data, ensure_ascii=False), '/episodes/0')
        packet, summary = self.build(max_bytes=10000)
        selected = packet['payload'][-1]['value']
        self.assertEqual(selected, data['episodes'][0])
        self.assertLess(summary['bytes'], 10000)
        self.assertNotIn('unrelated', (self.job / 'packets/one.json').read_text())
        self.verify()

    def test_pointer_escapes_empty_keys_and_arrays(self):
        self.add_input('rules', 'rules.json', '{"a/b":{"~key":[{"":false},null]}}', '/a~1b/~0key/0/')
        packet, _ = self.build()
        self.assertIs(packet['payload'][-1]['value'], False)
        self.verify()

    def test_json_projection_preserves_arbitrary_precision_numbers(self):
        number = '123456789.12345678901234567890123456789'
        self.add_input('rules', 'numbers.json', '{"precise":' + number + '}', '/precise')
        self.build()
        packet = json.loads((self.job / 'packets/one.json').read_text(), parse_float=Decimal)
        self.assertEqual(packet['payload'][-1]['value'], Decimal(number))
        self.verify()

    def test_zero_and_negative_episode_ids_survive_build_and_verify_unchanged(self):
        for episode in [0, -7]:
            with self.subTest(episode=episode):
                self.request['episode'] = episode
                output = 'packets/episode-' + str(episode) + '.json'
                packet, _ = self.build(output=output)
                self.assertIs(type(packet['episode']), int)
                self.assertEqual(packet['episode'], episode)
                self.assertEqual(packet['request']['episode'], episode)
                self.verify(packet=output)

    def test_budget_refuses_write_and_reports_kind_sizes_then_can_be_raised(self):
        self.add_input('rules', 'large.txt', '界' * 50000)
        error = self.build(ok=False)
        self.assertIn('max-bytes', error)
        self.assertIn('rules', error)
        self.assertFalse((self.job / 'packets/one.json').exists())
        packet, summary = self.build(max_bytes=200000)
        self.assertEqual(packet['maxBytes'], 200000)
        self.assertLessEqual(summary['bytes'], packet['maxBytes'])

    def test_stale_hash_rejected_before_write(self):
        (self.job / 'source.txt').write_text('被改写', encoding='utf-8')
        self.assertIn('sha256', self.build(ok=False))
        self.assertFalse((self.job / 'packets/one.json').exists())

    def test_verify_rejects_changed_reference_even_if_selected_value_unchanged(self):
        self.add_input('rules', 'rules.json', '{"selected":"ok","other":1}', '/selected')
        self.build()
        (self.job / 'rules.json').write_text('{"selected":"ok","other":2}')
        self.verify(ok=False)

    def test_verify_rejects_changed_manifest(self):
        self.build()
        self.request['episode'] = 2
        self.save_request()
        self.verify(ok=False)

    def test_verify_rejects_payload_or_metadata_tampering(self):
        packet, _ = self.build()
        target = self.job / 'packets/one.json'
        original = target.read_bytes()
        variants = [('payloadBytes', 0), ('episode', 2), ('maxBytes', 999999),
                    ('kindBytes', {}), ('payloadSha256', '0' * 64)]
        for key, value in variants:
            with self.subTest(key=key):
                modified = dict(packet)
                modified[key] = value
                target.write_text(json.dumps(modified, ensure_ascii=False), encoding='utf-8')
                self.verify(ok=False)
                target.write_bytes(original)
        packet['payload'][0]['value'] = '篡改对白'
        target.write_text(json.dumps(packet, ensure_ascii=False), encoding='utf-8')
        self.verify(ok=False)

    def test_verify_rejects_unknown_or_duplicate_packet_fields(self):
        self.build()
        target = self.job / 'packets/one.json'
        raw = target.read_text()
        target.write_text(raw[:-1] + ',"unknown":true}')
        self.verify(ok=False)
        target.write_text(raw[:-1] + ',"episode":1}')
        self.verify(ok=False)

    def test_different_sources_never_share_content(self):
        first, _ = self.build()
        self.request['inputs'][0]['sha256'] = digest('另一剧原文'.encode('utf-8'))
        (self.job / 'source.txt').write_text('另一剧原文', encoding='utf-8')
        second, _ = self.build(output='packets/two.json')
        self.assertEqual(first['payload'][0]['value'], self.original)
        self.assertEqual(second['payload'][0]['value'], '另一剧原文')
        self.assertNotEqual(first['payloadSha256'], second['payloadSha256'])
        self.verify(ok=False)
        self.verify(packet='packets/two.json')

    def test_existing_output_and_source_are_never_overwritten(self):
        self.build()
        target = self.job / 'packets/one.json'
        original = target.read_bytes()
        self.build(ok=False)
        self.assertEqual(target.read_bytes(), original)
        self.build(output='source.txt', ok=False)
        self.assertEqual((self.job / 'source.txt').read_bytes(), self.original.encode('utf-8'))

    def test_rejects_paths_outside_job_and_empty_paths(self):
        for path in ['', '../outside.txt', str(self.job / 'source.txt')]:
            with self.subTest(path=path):
                self.request['inputs'][0]['path'] = path
                self.build(ok=False)

    def test_rejects_input_output_and_manifest_symlink_escape(self):
        outside = self.root / 'outside'
        outside.mkdir()
        (outside / 'source.txt').write_bytes(self.original.encode('utf-8'))
        (self.job / 'escape').symlink_to(outside, target_is_directory=True)
        self.request['inputs'][0]['path'] = 'escape/source.txt'
        self.build(ok=False)
        self.request['inputs'][0]['path'] = 'source.txt'
        self.build(output='escape/packet.json', ok=False)
        self.assertFalse((outside / 'packet.json').exists())
        self.save_request()
        (outside / 'request.json').write_bytes((self.job / 'request.json').read_bytes())
        result = self.cli('build', '--workdir', self.job, '--manifest', 'escape/request.json',
                          '--output', 'packets/one.json')
        self.assertNotEqual(result.returncode, 0)

    def test_required_kinds_roles_and_explicit_history_kinds(self):
        original = json.loads(json.dumps(self.request))
        for kind in ['source', 'direction', 'state', 'assets', 'entry']:
            with self.subTest(missing=kind):
                self.request = dict(original, inputs=[x for x in original['inputs'] if x['kind'] != kind])
                self.build(ok=False)
        for kind in ['history', 'chat', 'transcript']:
            with self.subTest(kind=kind):
                self.request = json.loads(json.dumps(original))
                self.request['inputs'][0]['kind'] = kind
                self.build(ok=False)
        self.request = original
        self.request['role'] = 'reviewer'
        self.build(ok=False)
        self.add_input('draft', 'draft.txt', '待审完整采用稿')
        self.build()
        self.verify()

    def test_invalid_pointers_never_silently_choose_another_value(self):
        item = self.add_input('rules', 'rules.json', '{"items":[1,2],"scalar":3}', '/items/0')
        for pointer in ['items/0', '#/items/0', '/items/-', '/items/01', '/items/-1',
                        '/items/2', '/missing', '/scalar/key', '/~2', 3, None]:
            with self.subTest(pointer=pointer):
                item['pointer'] = pointer
                self.build(ok=False)

    def test_ambiguous_or_non_utf8_data_is_rejected_without_leaking_content(self):
        item = self.add_input('rules', 'bad.json', '{"secret":1,"secret":2}', '')
        for raw in [b'{"secret":1,"secret":2}', b'{"secret":NaN}', b'not JSON secret', b'\xff']:
            with self.subTest(raw=raw):
                (self.job / 'bad.json').write_bytes(raw)
                item['sha256'] = digest(raw)
                error = self.build(ok=False)
                self.assertNotIn('secret', error)

    def test_manifest_schema_requires_hash_integer_episode_and_known_fields(self):
        original = json.loads(json.dumps(self.request))
        variants = [('version', 'other'), ('episode', True), ('episode', '1'),
                    ('role', 'other'), ('inputs', []), ('unknown', 1)]
        for key, value in variants:
            with self.subTest(key=key, value=value):
                self.request = dict(original, **{key: value})
                self.build(ok=False)
        self.request = original
        del self.request['inputs'][0]['sha256']
        self.build(ok=False)


if __name__ == '__main__':
    unittest.main()
