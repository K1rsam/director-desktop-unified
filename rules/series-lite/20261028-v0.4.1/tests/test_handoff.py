"""Workspace transfer integration; receipt internals belong to execution tests."""
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'workspace.py'
sys.path.insert(0, str(SCRIPT.parent))
WORKSPACE = runpy.run_path(str(SCRIPT))


class HandoffTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.job = self.base / 'job'
        source = self.base / 'source.txt'
        source.write_text('第1集：原声与前集实际出口。', encoding='utf-8')
        extracted = self.cli('extract', '--source', source, '--output-dir', self.job)
        index = self.base / 'index.json'
        index.write_text(json.dumps({'version': 'series-source-v1', 'sourceHash': extracted['sha256'],
            'episodes': [{'number': n, 'scenes': [{'id': f'{n}-1'}]} for n in range(1, 5)]}), encoding='utf-8')
        self.cli('init', '--workdir', self.job, '--index', index, '--workers', 1)
        self.save('draft-v1.json', {'source': 'complete candidate; keep immutable', 'duration': 4.125})
        self.save('author-notes.json', {'covered': ['1-1'], 'pending': 'whole-episode finishing'})

    def cli(self, *args, ok=True):
        result = subprocess.run([sys.executable, str(SCRIPT), *map(str, args)],
                                capture_output=True, text=True, encoding='utf-8')
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        return json.loads(result.stdout) if ok else result.stderr

    def save(self, name, data):
        path = self.job / name
        path.write_text(json.dumps(data, ensure_ascii=False), encoding='utf-8')
        return path

    def state(self):
        return json.loads((self.job / 'state.json').read_text(encoding='utf-8'))

    def claim(self, owner='author', group=1, ok=True):
        return self.cli('claim', '--workdir', self.job, '--group', group, '--owner', owner, ok=ok)

    def transfer(self, owner='author', target='finisher', n=1, candidate='draft-v1.json',
                 evidence='author-notes.json', ok=True):
        return self.cli('handoff', '--workdir', self.job, '--episode', n, '--owner', owner,
                        '--to', target, '--candidate', candidate, '--evidence', evidence, ok=ok)

    def evidence(self, n=1, adoption=False):
        state = self.state()
        self.save(f'adopted-{n}.json', {'fixture': 'finished episode', 'duration': 4.125})
        result = {'version': 'series-episode-evidence-v1', 'episode': n,
                  'sourceHash': state['sourceHash'], 'sourceIndexHash': state['sourceIndexHash'],
                  'scenes': [f'{n}-1'], 'artifacts': {stage: f'adopted-{n}.json'
                      for stage in ('intake', 'assets', 'direction', 'delivery')},
                  'adopted': f'adopted-{n}.json', 'reviewMode': 'self'}
        if state['episodes'][str(n)].get('claimedUpstreamHash'):
            result['entryReview'] = {'upstreamHash': state['episodes'][str(n)]['claimedUpstreamHash'],
                                    'notes': 'fixture: read the preserved actual predecessor'}
        if adoption:
            result['adoptionReceipt'] = 'receipt.json'
            self.save('receipt.json', {'fixture': 'receipt verification is mocked in workspace unit tests'})
        return self.save(f'evidence-{n}.json', result)

    def verifier(self, n=1):
        return Mock(return_value={'episode': n,
                    'bundle': WORKSPACE['file_record'](self.job, f'adopted-{n}.json')})

    def test_normal_transfer_then_complete_preserves_evidence_and_authorship(self):
        self.claim()
        result = self.transfer()
        self.assertFalse(result['reusedHandoff'])
        state = self.state()
        self.assertEqual(state['episodes']['1']['status'], 'in_flight')
        self.assertEqual(state['episodes']['1']['owner'], 'finisher')
        self.assertEqual(state['dispatch']['workers']['author']['claims'], [1])
        self.assertEqual(state['dispatch']['workers']['finisher']['claims'], [])
        self.assertEqual(state['dispatch']['workers']['finisher']['handoffs'], [1])
        self.assertEqual(state['history'], [])
        evidence = self.evidence(adoption=True)
        self.cli('complete', '--workdir', self.job, '--episode', 1,
                 '--owner', 'author', '--evidence', evidence, ok=False)
        verify = self.verifier()
        with patch.dict(sys.modules, {'episode_execution': SimpleNamespace(verify_adoption=verify)}):
            result = WORKSPACE['complete'](self.job, state, 1, 'finisher', str(evidence))
        self.assertTrue(result['episodes']['1']['evidenceValid'])
        self.assertEqual(state['episodes']['1']['completion']['reviewMode'], 'self')
        self.assertIn(WORKSPACE['file_record'](self.job, 'draft-v1.json'), state['episodes']['1']['completion']['files'])
        self.assertTrue(verify.called)
        for call in verify.call_args_list:
            self.assertEqual(call.args, (self.job, 'receipt.json'))
            self.assertEqual(call.kwargs['episode'], 1)
            self.assertLessEqual(set(call.kwargs), {'episode', 'cache'})

    def test_repeated_identical_handoff_is_idempotent(self):
        self.claim()
        first = self.transfer()
        before = (self.job / 'state.json').read_bytes()
        second = self.transfer()
        self.assertTrue(second['reusedHandoff'])
        self.assertEqual(first['handoffHash'], second['handoffHash'])
        self.assertEqual((self.job / 'state.json').read_bytes(), before)
        self.assertEqual(len(self.state()['episodes']['1']['handoffs']), 1)

    def test_conflicting_and_expired_retries_are_rejected(self):
        self.claim()
        self.transfer()
        self.save('different.json', {'fixture': 'new candidate'})
        self.assertIn('冲突', self.transfer(candidate='different.json', ok=False))
        self.transfer('finisher', 'second_finisher')
        before = (self.job / 'state.json').read_bytes()
        self.assertIn('过期', self.transfer(ok=False))
        self.assertEqual((self.job / 'state.json').read_bytes(), before)

    def test_candidate_and_basis_must_remain_immutable(self):
        for changed in ('draft-v1.json', 'author-notes.json'):
            with self.subTest(changed=changed):
                if not self.state()['episodes']['1'].get('owner'):
                    self.claim()
                    self.transfer()
                saved = (self.job / changed).read_bytes()
                self.save(changed, {'fixture': 'changed after transfer'})
                self.assertIn('冻结交接', self.transfer(ok=False))
                evidence = self.evidence(adoption=True)
                with self.assertRaisesRegex(ValueError, '冻结交接'):
                    WORKSPACE['complete'](self.job, self.state(), 1, 'finisher', str(evidence))
                (self.job / changed).write_bytes(saved)

    def test_old_owner_cannot_transfer_or_block_and_same_owner_cannot_receive(self):
        self.claim()
        self.assertIn('不同', self.transfer(target='author', ok=False))
        self.transfer()
        self.assertIn('当前owner', self.transfer(target='another', ok=False))
        self.cli('block', '--workdir', self.job, '--episode', 1, '--owner', 'author',
                 '--evidence', 'author-notes.json', ok=False)

    def test_transfer_replaces_one_slot_and_does_not_allow_a_second_job(self):
        state = self.state()
        state['groups'] = [{'id': n, 'episodes': [n]} for n in range(1, 5)]
        for n in range(1, 5):
            state['episodes'][str(n)]['group'] = n
        self.save('state.json', state)
        self.claim('author', 1)
        self.claim('busy', 2)
        self.claim('third', 3)
        self.assertIn('执行中', self.transfer(target='busy', ok=False))
        self.transfer()  # all three child slots are occupied, but this replaces one
        self.assertEqual(len([ep for ep in self.state()['episodes'].values() if ep['status'] == 'in_flight']), 3)
        self.assertIn('槽', self.claim('fourth', 4, ok=False))

    def test_rejects_coordinator_even_for_legacy_workspaces(self):
        self.claim()
        for legacy in (False, True):
            if legacy:
                state = self.state()
                state.pop('dispatch')
                self.save('state.json', state)
            for target in ('/root', 'root', 'main', 'coordinator'):
                self.assertIn('主控', self.transfer(target=target, ok=False))
        self.transfer()
        self.assertNotIn('dispatch', self.state())

    def test_observations_are_retained_and_rotation_cannot_be_bypassed(self):
        self.claim()
        self.transfer()
        observation = self.save('context.json', {'version': 'series-context-observation-v1',
            'owner': 'finisher', 'inputTokens': 70000, 'compactions': 0, 'basis': 'fixture host observation'})
        self.cli('observe', '--workdir', self.job, '--owner', 'finisher', '--evidence', observation)
        self.transfer('finisher', 'fresh')
        self.assertIn('轮换', self.transfer('fresh', 'finisher', ok=False))
        self.assertEqual(len(self.state()['dispatch']['workers']['finisher']['observations']), 1)

    def test_handoff_does_not_append_claim_but_counts_actual_context_episodes(self):
        self.claim()
        self.transfer()
        state = self.state()
        state['dispatch']['batchSize'] = 1
        # Releasing the fixture task isolates the next-episode budget gate.
        state['episodes']['1']['status'] = 'pending'
        with self.assertRaisesRegex(ValueError, '批次上限'):
            WORKSPACE['check_worker'](state, 'finisher')
        WORKSPACE['check_worker'](state, 'finisher', replacing=1)
        self.assertEqual(state['dispatch']['workers']['finisher']['claims'], [])

    def test_bound_predecessor_survives_transfer_and_changed_predecessor_is_rejected(self):
        self.claim()
        evidence = self.evidence()
        self.cli('complete', '--workdir', self.job, '--episode', 1, '--owner', 'author', '--evidence', evidence)
        self.claim()
        binding = self.state()['episodes']['2']['claimedUpstreamHash']
        self.transfer(n=2)
        self.assertEqual(self.state()['episodes']['2']['claimedUpstreamHash'], binding)
        self.assertEqual(self.state()['episodes']['2']['handoffs'][0]['claimedUpstreamHash'], binding)
        evidence = self.evidence(2, adoption=True)
        state = self.state()
        with patch.dict(sys.modules, {'episode_execution': SimpleNamespace(verify_adoption=self.verifier(2))}):
            result = WORKSPACE['complete'](self.job, state, 2, 'finisher', str(evidence))
        self.assertEqual(result['episodes']['2']['boundary'], 'ACTUAL_RECONCILED')
        self.save('adopted-1.json', {'fixture': 'changed actual predecessor'})
        self.assertIn('前集采用稿变化', self.transfer(n=2, ok=False))

    def test_complete_requires_receipt_and_checks_bundle_binding(self):
        self.claim()
        self.transfer()
        evidence = self.evidence()
        self.assertIn('adoptionReceipt', self.cli('complete', '--workdir', self.job, '--episode', 1,
                      '--owner', 'finisher', '--evidence', evidence, ok=False))
        evidence = self.evidence(adoption=True)
        verify = Mock(return_value={'episode': 1, 'bundle': {'path': 'different.json', 'sha256': 'bad'}})
        with patch.dict(sys.modules, {'episode_execution': SimpleNamespace(verify_adoption=verify)}):
            with self.assertRaisesRegex(ValueError, 'adopted不同'):
                WORKSPACE['complete'](self.job, self.state(), 1, 'finisher', str(evidence))

    def test_receipt_reverification_exposes_dependency_drift_in_status(self):
        self.claim()
        self.transfer()
        evidence = self.evidence(adoption=True)
        state = self.state()
        verify = self.verifier()
        with patch.dict(sys.modules, {'episode_execution': SimpleNamespace(verify_adoption=verify)}):
            WORKSPACE['complete'](self.job, state, 1, 'finisher', str(evidence))
            verify.side_effect = ValueError('fixture: adopted dependency changed')
            self.assertFalse(WORKSPACE['view'](self.job, state)['episodes']['1']['evidenceValid'])

    def test_view_verifies_each_episode_once_and_next_view_refreshes(self):
        state = self.state()
        for n in (1, 2):
            evidence = self.evidence(n, adoption=True)
            adopted = WORKSPACE['file_record'](self.job, f'adopted-{n}.json')
            evidence_record = WORKSPACE['file_record'](self.job, str(evidence))
            receipt = WORKSPACE['file_record'](self.job, 'receipt.json')
            state['episodes'][str(n)].update(status='complete', completion={
                'evidence': evidence_record, 'adopted': adopted, 'adoptionReceipt': receipt,
                'reviewMode': 'self', 'files': [evidence_record, adopted, receipt]})
        state['episodes']['2']['boundaryRecord'] = {
            'upstreamHash': state['episodes']['1']['completion']['adopted']['sha256'],
            'currentHash': state['episodes']['2']['completion']['adopted']['sha256']}
        caches = []
        def checked(root, path, episode=None, cache=None):
            caches.append(cache)
            return {'bundle': state['episodes'][str(episode)]['completion']['adopted']}
        verify = Mock(side_effect=checked)
        with patch.dict(sys.modules, {'episode_execution': SimpleNamespace(verify_adoption=verify)}):
            result = WORKSPACE['view'](self.job, state)
            self.assertEqual(result['episodes']['2']['boundary'], 'ACTUAL_RECONCILED')
            self.assertEqual([call.kwargs['episode'] for call in verify.call_args_list], [1, 2])
            self.assertIs(caches[0], caches[1])
            prior_cache = caches[0]
            verify.reset_mock()
            WORKSPACE['view'](self.job, state)
            self.assertEqual(verify.call_count, 2)
            self.assertIsNot(caches[-1], prior_cache)

    def test_resume_keeps_handoff_history_and_old_transfer_cannot_replay(self):
        self.claim()
        self.transfer()
        self.save('recovery.json', {'fixture': 'real interruption; stopped finisher, inspected retained draft'})
        self.cli('resume', '--workdir', self.job, '--episode', 1, '--evidence', 'recovery.json')
        self.assertEqual(len(self.state()['history'][0]['prior']['handoffs']), 1)
        self.claim()
        evidence = self.evidence()
        self.assertIn('adoptionReceipt', self.cli('complete', '--workdir', self.job, '--episode', 1,
                      '--owner', 'author', '--evidence', evidence, ok=False))
        self.assertIn('恢复前', self.transfer(ok=False))
        self.save('new-notes.json', {'fixture': 'inspected actual interruption; new responsibility'})
        self.transfer(evidence='new-notes.json')

    def test_candidate_path_escape_and_duplicate_are_rejected(self):
        self.claim()
        outside = self.base / 'outside.json'
        outside.write_text('{}', encoding='utf-8')
        self.assertIn('逃逸', self.transfer(candidate=str(outside), ok=False))
        self.cli('handoff', '--workdir', self.job, '--episode', 1, '--owner', 'author', '--to', 'finisher',
                 '--candidate', 'draft-v1.json', '--candidate', './draft-v1.json', '--evidence', 'author-notes.json', ok=False)

    def test_concurrent_identical_transfer_succeeds_once_and_reuses_same_record(self):
        self.claim()
        command = [sys.executable, str(SCRIPT), 'handoff', '--workdir', str(self.job), '--episode', '1',
                   '--owner', 'author', '--to', 'finisher', '--candidate', 'draft-v1.json',
                   '--evidence', 'author-notes.json']
        processes = [subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(3)]
        results = [json.loads(process.communicate()[0]) for process in processes]
        self.assertTrue(all(process.returncode == 0 for process in processes))
        self.assertEqual(sum(not result['reusedHandoff'] for result in results), 1)
        self.assertEqual(len(self.state()['episodes']['1']['handoffs']), 1)


if __name__ == '__main__':
    unittest.main()
