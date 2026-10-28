"""Offline scheduling tests; no agent or model calls."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'workspace.py'


class DispatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.job = self.base / 'job'
        source = self.base / 'source.txt'
        source.write_text('旧伤仍在；秘密只有甲知道。', encoding='utf-8')
        self.call('extract', '--source', source, '--output-dir', self.job)
        manifest = json.loads((self.job / 'extraction.json').read_text())
        self.index = self.base / 'index.json'
        self.index.write_text(json.dumps({'version': 'series-source-v1', 'sourceHash': manifest['sha256'],
            'episodes': [{'number': n, 'scenes': [{'id': f'{n}-1'}]} for n in range(1, 13)]}))

    def call(self, *args, ok=True):
        p = subprocess.run([sys.executable, str(SCRIPT), *map(str, args)], capture_output=True, text=True)
        self.assertEqual(p.returncode == 0, ok, p.stdout + p.stderr)
        return json.loads(p.stdout) if ok else p.stderr

    def init(self, *args):
        return self.call('init', '--workdir', self.job, '--index', self.index, '--full', *args)

    def claim(self, owner, group=1, ok=True):
        return self.call('claim', '--workdir', self.job, '--group', group, '--owner', owner, ok=ok)

    def complete(self, n, owner):
        state = json.loads((self.job / 'state.json').read_text())
        f = self.job / f'ep{n}.txt'
        f.write_text('test fixture: checked source, old injury, private knowledge and actual exit.')
        evidence = {'version': 'series-episode-evidence-v1', 'episode': n,
            'sourceHash': state['sourceHash'], 'sourceIndexHash': state['sourceIndexHash'],
            'scenes': [f'{n}-1'], 'artifacts': {k: [f.name] for k in ('intake', 'assets', 'direction', 'delivery')},
            'adopted': f.name, 'reviewMode': 'self'}
        if state['episodes'][str(n)].get('claimedUpstreamHash'):
            evidence['entryReview'] = {'upstreamHash': state['episodes'][str(n)]['claimedUpstreamHash'],
                                      'notes': 'test fixture: checked actual predecessor'}
        path = self.job / f'ep{n}-evidence.json'
        path.write_text(json.dumps(evidence))
        return self.call('complete', '--workdir', self.job, '--episode', n,
                         '--owner', owner, '--evidence', path)

    def test_default_reserves_coordinator_slot_and_rejects_main_authorship(self):
        state = self.init()
        self.assertEqual(len(state['groups']), 3)
        for name in ('main', 'root', '/root', 'coordinator'):
            self.assertIn('主控', self.claim(name, ok=False))
        self.assertEqual(self.claim('/root/author_01')['episode'], 1)

    def test_fresh_worker_continues_after_two_episodes_without_losing_predecessor(self):
        self.init('--workers', 1)
        for n in (1, 2):
            self.assertEqual(self.claim('author_01')['episode'], n)
            self.complete(n, 'author_01')
        self.assertIn('轮换', self.claim('author_01', ok=False))
        self.assertEqual(self.claim('author_02')['episode'], 3)
        state = json.loads((self.job / 'state.json').read_text())
        self.assertEqual(state['episodes']['3']['claimedUpstreamHash'],
                         state['episodes']['2']['completion']['adopted']['sha256'])
        self.complete(3, 'author_02')

    def test_owner_cannot_claim_two_groups_and_capacity_does_not_count_root_as_author(self):
        self.init('--workers', 4)
        self.claim('a', 1)
        self.assertIn('执行中', self.claim('a', 2, ok=False))
        self.claim('b', 2)
        self.claim('c', 3)
        self.assertIn('槽', self.claim('d', 4, ok=False))

    def test_five_host_slots_allow_four_children_but_never_main_authorship(self):
        state = self.init('--active-slots', 5)
        self.assertEqual(len(state['groups']), 4)
        for group in range(1, 5):
            self.assertIsNotNone(self.claim(f'child_{group}', group)['episode'])

    def test_unknown_observation_does_not_become_zero_or_reset_rotation(self):
        self.init('--workers', 1)
        self.claim('a')
        for index, compactions in enumerate((1, 0)):
            report = self.job / f'context-{index}.json'
            report.write_text(json.dumps({'version': 'series-context-observation-v1', 'owner': 'a',
                'inputTokens': None, 'compactions': compactions, 'basis': 'host observation; tokens unavailable'}))
            result = self.call('observe', '--workdir', self.job, '--owner', 'a', '--evidence', report)
            self.assertIsNone(result['inputTokens'])
            self.assertTrue(result['rotationRequired'])

    def test_context_compaction_requires_rotation_at_saved_boundary(self):
        self.init('--workers', 1)
        self.claim('a')
        report = self.job / 'context.json'
        report.write_text(json.dumps({'version': 'series-context-observation-v1', 'owner': 'a',
            'inputTokens': None, 'compactions': 1, 'basis': 'host reported one compaction'}))
        self.call('observe', '--workdir', self.job, '--owner', 'a', '--evidence', report)
        self.complete(1, 'a')
        self.assertIn('轮换', self.claim('a', ok=False))
        self.assertEqual(self.claim('b')['episode'], 2)

    def test_high_measured_input_and_unknown_usage_are_distinct(self):
        self.init('--workers', 1)
        self.claim('a')
        report = self.job / 'context.json'
        report.write_text(json.dumps({'version': 'series-context-observation-v1', 'owner': 'a',
            'inputTokens': 70000, 'compactions': 0, 'basis': 'last native response input_tokens'}))
        self.call('observe', '--workdir', self.job, '--owner', 'a', '--evidence', report)
        self.complete(1, 'a')
        self.assertIn('轮换', self.claim('a', ok=False))

    def test_compact_status_omits_history_and_completed_artifact_bodies(self):
        self.init('--workers', 1)
        self.claim('a')
        self.complete(1, 'a')
        status = self.call('status', '--workdir', self.job, '--compact')
        self.assertNotIn('history', status)
        self.assertNotIn('episodes', status)
        self.assertEqual(status['counts']['complete'], 1)
        self.assertEqual(status['nextByGroup'][0]['episode'], 2)
        self.assertFalse(status['readyForDelivery'])

    def test_legacy_state_is_readable_and_not_silently_migrated(self):
        self.init('--workers', 1)
        state_path = self.job / 'state.json'
        state = json.loads(state_path.read_text())
        state.pop('dispatch', None)
        state_path.write_text(json.dumps(state))
        self.call('status', '--workdir', self.job)
        self.assertNotIn('dispatch', json.loads(state_path.read_text()))


if __name__ == '__main__':
    unittest.main()
