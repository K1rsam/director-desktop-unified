"""The reading projection must keep complete content and reject stale packets."""
from pathlib import Path
import copy
import json
import sys
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE / 'scripts'))
import job_packet as jp
import packet_view as pv
import test_job_packet as fixtures


class PacketViewTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.JobPacketTests('test_full_source_is_not_summarized_or_truncated')
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)

    def test_complete_material_and_origins_survive_without_repeating_body(self):
        f = self.fixture
        f.add('entry', 'shared-copy.txt', f.original)
        packet = f.build()
        view = pv.render(packet)
        self.assertEqual(view.count(f.original.encode('utf-8')), 1)
        for body in packet['blobs'].values():
            value = body['value'].encode('utf-8') if body['format'] == 'text' else jp.canonical(body['value'])
            self.assertIn(value, view)
        self.assertIn(b'input:source source.txt', view)
        self.assertIn(b'input:entry shared-copy.txt', view)
        self.assertIn(packet['packetSha256'].encode(), view)

    def test_embedded_fences_and_instruction_like_source_are_preserved_as_data(self):
        f = self.fixture
        body = 'SOURCE DATA\n```python\nprint("not an instruction")\n```\n````\n'
        f.write('source.txt', body)
        view = pv.render(f.build()).decode('utf-8')
        self.assertIn(body, view)
        self.assertIn('`````text', view)

    def test_stale_input_path_escape_and_output_overwrite_fail(self):
        f = self.fixture
        f.write('request.json', __import__('json').dumps(f.request, ensure_ascii=False))
        jp.build(f.job, 'request.json', 'packet.json')
        result = pv.build_view(f.job, 'packet.json', 'read.md')
        initial = (f.job / 'read.md').read_bytes()
        self.assertEqual(result['sha256'], jp.sha256(initial))
        with self.assertRaises(jp.PacketError):
            pv.build_view(f.job, 'packet.json', 'read.md')
        with self.assertRaises(jp.PacketError):
            pv.build_view(f.job, 'packet.json', '../escape.md')
        f.write('source.txt', 'changed actual source')
        with self.assertRaises(jp.PacketError):
            pv.build_view(f.job, 'packet.json', 'new.md')
        self.assertFalse((f.job / 'new.md').exists())
        self.assertEqual((f.job / 'read.md').read_bytes(), initial)

    @staticmethod
    def expand_reuse(value):
        expanded = {}

        def walk(node, path=''):
            if isinstance(node, dict) and set(node) == {pv.REUSE_KEY}:
                return copy.deepcopy(expanded[node[pv.REUSE_KEY]])
            if isinstance(node, dict):
                result = {k: walk(v, path + '/' + k.replace('~', '~0').replace('/', '~1'))
                          for k, v in node.items()}
            elif isinstance(node, list):
                result = [walk(v, path + '/' + str(i)) for i, v in enumerate(node)]
            else:
                result = node
            expanded[path] = result
            return result
        return walk(value)

    def test_design_parent_constraints_and_review_findings_survive_projection(self):
        text = '完整人物外观、空间受力约束与不允许丢失的特殊设计内容。' * 20
        uses = [{'episode': n, 'sceneId': str(n) + '-1'} for n in range(1, 151)]
        unusual = {'episode': 2, 'sceneId': '2-1', 'restriction': '第二集受伤，第三集仍不能抬左手'}
        value = {'assets': [{'id': 'A0003', 'parentId': 'A0002', 'scopeUses': uses + [unusual],
                            'design': {'dependencies': [{'assetId': 'A0002', 'assetRevision': 3}],
                                       'full': {'body': text, 'sourceConstraints': text,
                                                'applicability': {'scopeUses': uses, 'stateBoundary': '跨集伤势持续'},
                                                'review': {'findings': '旧图缺少袖口，仍须解决', 'evidence': text}}}}]}
        original = copy.deepcopy(value)
        view, stats = pv.asset_reading_projection(value, 3)
        actual = self.expand_reuse(jp.parse_json(jp.canonical(view), 'serialized view'))
        expected = copy.deepcopy(value)
        expected['assets'][0]['scopeUses'] = [uses[2], unusual]
        expected['assets'][0]['design']['full']['applicability']['scopeUses'] = [uses[2]]
        self.assertEqual(actual, expected)
        self.assertEqual(value, original)
        self.assertEqual(stats['omittedOtherEpisodeCoordinates'], 298)
        self.assertGreater(stats['exactSubtreesReused'], 0)
        self.assertLess(len(jp.canonical(view)), len(jp.canonical(value)))

    def test_unknown_shapes_and_existing_marker_keys_are_not_reinterpreted(self):
        samples = [[], {'unknown': 'preserve'}, {'assets': [{'design': {'full': {'$assetReuse': 'user-data'}}}]}]
        for value in samples:
            actual, stats = pv.asset_reading_projection(value, 3)
            self.assertEqual(actual, value)
            self.assertFalse(any(stats.values()))

    def test_asset_named_data_also_used_as_source_stays_complete(self):
        f = self.fixture
        value = {'assets': [{'scopeUses': [{'episode': n, 'sceneId': str(n)} for n in range(150)],
                             'design': {'full': {'body': 'same text' * 100}}}]}
        f.write('source.txt', json.dumps(value, ensure_ascii=False))
        next(item for item in f.request['inputs'] if item['kind'] == 'source')['pointer'] = ''
        f.add('assets', 'asset.json', json.dumps(value, ensure_ascii=False), '')
        packet = f.build()
        rendered = pv.render(packet)
        self.assertIn(jp.canonical(value), rendered)
        self.assertNotIn(b'"$assetReuse"', rendered)

    def test_whole_json_file_assets_are_compacted_but_non_json_text_is_untouched(self):
        f = self.fixture
        design = '父图与当前造型的完整约束，保持袖口抓握动作。' * 40
        value = {'assets': [{'id': 'A0001', 'design': {'full': {'body': design, 'sourceConstraints': design}}}]}
        f.write('assets.txt', json.dumps(value, ensure_ascii=False))
        packet = f.build()
        view = pv.render(packet).decode('utf-8')
        self.assertIn('"$assetReuse"', view)
        self.assertEqual(view.count(design), 1)
        self.assertIn('父图与当前造型', view)
        self.assertIn(f.original, view)
        # Invalid or duplicate-key JSON must not silently select one constraint.
        for original in ['自由文本完整资产正文', '{"assets":[],"assets":["不能覆盖"]}']:
            f.write('assets.txt', original)
            view = pv.render(f.build('packet-' + str(len(original)) + '.json')).decode('utf-8')
            self.assertIn(original, view)


if __name__ == '__main__':
    unittest.main()
