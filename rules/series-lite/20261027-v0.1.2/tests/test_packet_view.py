"""The reading projection must keep complete content and reject stale packets."""
from pathlib import Path
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


if __name__ == '__main__':
    unittest.main()
