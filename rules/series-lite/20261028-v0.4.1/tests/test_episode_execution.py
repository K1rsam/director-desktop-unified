"""Real local CLI checks for episode adoption, interruption recovery and handoff."""
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


BASE = Path(__file__).resolve().parents[1]
SCRIPTS = BASE / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import episode_execution as execution
import workspace


class EpisodeExecutionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='episode-execution-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.job = self.root / 'job'
        self.scripts = SCRIPTS
        self.bundle = json.loads((BASE / 'tests/fixtures/lingmo-v7-bundle.json').read_text(encoding='utf-8'))
        source_text = '\n'.join(row['text'] for row in self.bundle['episodes'][0]['source']['sourceRows'])
        source = self.root / 'source.txt'
        source.write_text(source_text, encoding='utf-8')
        self.source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
        self.cli('workspace.py', 'extract', '--source', source, '--output-dir', self.job)
        self.save('index.json', {'version': 'series-source-v1', 'sourceHash': self.source_hash,
                               'episodes': [{'number': 1, 'scenes': [{'id': '1-1'}, {'id': '1-2'}]}]})
        self.cli('workspace.py', 'init', '--workdir', self.job, '--index', self.job / 'index.json',
                 '--workers', '1', '--full')
        self.bundle['testFixture'] = False
        self.bundle['sourceHash'] = self.source_hash
        assets = [{**asset, 'aliases': [],
                   'origin': {'kind': 'source', 'sourceRefs': ['E01:R1'], 'basis': '合成测试源中的人物、场景和持物。'},
                   'design': {'state': 'TEXT_ADOPTED', 'text': '完整采用设计：保持源中的身份、关闭状态和持物关系。'}}
                  for asset in self.bundle['assets']]
        self.save('seed.json', {'sourceHash': self.source_hash, 'importProfile': 'lingmo-v7', 'assets': assets})
        initialized = self.cli('assets.mjs', 'init', '--registry', self.job / 'registry', '--input', self.job / 'seed.json')
        registry = self.registry()
        self.bundle['assetRegistry'] = {'id': initialized['id'], 'revision': initialized['revision']}
        self.bundle['assets'] = [{**{k: v for k, v in asset.items() if k not in ('id', 'revision')},
                                  'assetId': asset['id'], 'assetRevision': asset['revision']}
                                 for asset in registry['assets']]
        episode = self.bundle['episodes'][0]
        self.baseline = json.loads((BASE / 'tests/fixtures/visual-baseline.json').read_text(encoding='utf-8'))
        episode['settings']['visualBaseline'] = copy.deepcopy(self.baseline)
        episode['settings']['visualStyle'] = self.baseline['visualStyle']
        self.save('visual-baseline.json', self.baseline)
        self.save('intake.json', {'sourceHash': self.source_hash,
                                 'episodes': [{'number': 1, 'source': episode['source']}]})
        frozen = self.cli('storyboard.mjs', 'freeze-source', '--input', self.job / 'intake.json',
                          '--output', self.job / 'source-lock.json')
        self.bundle['sourceLockHash'] = frozen['sourceLockHash']
        self.bundle['generationTextVersion'] = 'voice-delivery-v1'
        self.bundle['generationBoundaryVersion'] = 'video-prompt-v1'
        self.save('bundle.json', self.bundle)
        for kind in ('source', 'plan', 'selection', 'draft'):
            self.save(kind + '.json', episode[kind])
        for name, text in [('direction', '冷色写实；面罩遮住口鼻，保留全部原声和准确持物关系。'),
                           ('facts', '完整本集事实：闭门；钥匙始终在右手；走廊为空。'),
                           ('entry', '全剧首集，源起点为实际入口。'),
                           ('review', '合成测试：已逐项核对完整源、必要设计、计划、选择、全部稿和首集入口。')]:
            self.save(name + '.txt', text)
        self.packet_manifest = {
            'version': 'series-job-request-v1', 'episode': 1, 'role': 'finisher',
            'review': {'path': 'review.txt'},
            'inputs': [{'kind': kind, 'path': kind + '.json'} for kind in ('source', 'plan', 'selection', 'draft')]
                      + [{'kind': 'direction', 'path': 'direction.txt'},
                         {'kind': 'visual-baseline', 'path': 'visual-baseline.json'},
                         {'kind': 'state', 'path': 'facts.txt'},
                         {'kind': 'entry', 'path': 'entry.txt'},
                         {'kind': 'assets', 'path': 'registry/v000001.json'}]}
        self.build_packet()
        self.save('execution-request.json', {
            'version': 'series-episode-execution-v1', 'episode': 1,
            'packet': 'packet.json', 'bundle': 'bundle.json',
            'sourceLock': 'source-lock.json', 'registry': 'registry'})

    def save(self, name, value):
        path = self.job / name
        path.parent.mkdir(parents=True, exist_ok=True)
        text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False, indent=2)
        path.write_text(text, encoding='utf-8')
        return path

    def read(self, name):
        return json.loads((self.job / name).read_text(encoding='utf-8'))

    def cli(self, script, *args, ok=True):
        command = ['node' if script.endswith('.mjs') else sys.executable, str(self.scripts / script), *map(str, args)]
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8', timeout=180)
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        if ok:
            return json.loads(result.stdout)
        self.assertEqual(result.stdout, '')
        return result.stderr

    def registry(self):
        return self.cli('assets.mjs', 'status', '--registry', self.job / 'registry')['registry']

    def build_packet(self, output='packet.json'):
        self.save('packet-request.json', self.packet_manifest)
        return self.cli('job_packet.py', 'build', '--workdir', self.job,
                        '--manifest', 'packet-request.json', '--output', output)

    def run_execution(self, ok=True, output='adoption'):
        return self.cli('episode_execution.py', 'run', '--workdir', self.job,
                        '--manifest', 'execution-request.json', '--output', output, ok=ok)

    def test_new_execution_rejects_missing_boundary_marker(self):
        packet = self.read('packet.json')
        self.assertIn("generationBoundaryVersion:'video-prompt-v1'", packet['task']['contract'])
        for value in (None, 'unknown'):
            if value is None:
                self.bundle.pop('generationBoundaryVersion', None)
            else:
                self.bundle['generationBoundaryVersion'] = value
            self.save('bundle.json', self.bundle)
            self.assertIn('generationBoundaryVersion', self.run_execution(ok=False, output='boundary-' + str(value)))
        self.assertFalse((self.job / 'adoption/adoption.json').exists())

    def test_frozen_old_contract_without_marker_still_reads(self):
        import job_packet
        packet = self.read('packet.json')
        packet['task']['contract'] = 'Historical contract before generation boundary policy.'
        self.bundle.pop('generationBoundaryVersion')
        self.save('bundle.json', self.bundle)
        # Simulate the output of verified historical packet loading; this tests
        # the compatibility branch, not authenticity of an invented receipt.
        with patch.object(job_packet, 'load_frozen', return_value=packet):
            execution.inputs(self.job, 'execution-request.json', historical_runtime={})
        packet['task']['contract'] = "generationBoundaryVersion:'video-prompt-v1'"
        with patch.object(job_packet, 'load_frozen', return_value=packet):
            with self.assertRaisesRegex(ValueError, 'generationBoundaryVersion'):
                execution.inputs(self.job, 'execution-request.json', historical_runtime={})

    def test_adoption_rejects_rewritten_or_replaced_visual_baseline(self):
        original = copy.deepcopy(self.bundle)
        for field in ('visualStyle', 'visualBaseline'):
            with self.subTest(field=field):
                self.bundle = copy.deepcopy(original)
                settings = self.bundle['episodes'][0]['settings']
                if field == 'visualStyle':
                    settings[field] = '3D国漫'
                else:
                    settings[field]['revision'] = 2
                self.save('bundle.json', self.bundle)
                error = self.run_execution(ok=False, output='bad-' + field)
                self.assertIn('visual-baseline', error)
                self.assertFalse((self.job / ('bad-' + field) / 'adoption.json').exists())

    def verify_execution(self, ok=True):
        return self.cli('episode_execution.py', 'verify', '--workdir', self.job,
                        '--receipt', 'adoption/adoption.json', ok=ok)

    def asset_change(self, *, revise=False):
        registry = self.registry()
        if revise:
            asset = copy.deepcopy(registry['assets'][0])
            asset['design']['text'] += ' 修订面罩材质说明。'
        else:
            asset = {'name': '下一集备用盒', 'key': '下一集备用盒PR', 'type': 'PR', 'status': '常态', 'aliases': [],
                     'origin': {'kind': 'design', 'sourceRefs': ['E01:R1'], 'basis': '合成后续集备用道具，不用于本集。'},
                     'design': {'state': 'TEXT_ADOPTED', 'text': '黑色矩形盒，已采用完整文字设计。'}}
        change = {'version': 'series-asset-change-v1', 'baseRevision': registry['revision'],
                  'requestId': 'revision-' + str(registry['revision']), 'stage': 'asset-design',
                  'action': 'revise' if revise else 'add', 'asset': asset,
                  'reason': '合成测试所需独立修改', 'duplicateReview': '逐项核对总库身份，保留现有ID和状态。', 'uses': []}
        if revise:
            change['assetId'] = asset['id']
        path = self.save('change-' + str(registry['revision']) + '.json', change)
        return self.cli('assets.mjs', 'apply', '--registry', self.job / 'registry', '--input', path)

    def record_later_uses(self):
        registry = self.registry()
        request = {'version': 'series-asset-uses-v1', 'baseRevision': registry['revision'],
                   'requestId': 'later-source-uses', 'reason': '合成用途追加，不改变采用设计或适用限制',
                   'entries': [{'assetId': registry['assets'][0]['id'], 'assetRevision': registry['assets'][0]['revision'],
                                'uses': [{'episode': 2, 'sceneId': '2-1', 'purpose': '后续同版身份复用',
                                          'sourceRefs': ['synthetic-later-source:R1']}]}]}
        path = self.save('later-uses.json', request)
        return self.cli('assets.mjs', 'record-uses', '--registry', self.job / 'registry', '--input', path)

    def test_usage_annotation_keeps_real_adoption_bytes_and_reuses_all_steps(self):
        self.run_execution()
        receipt = (self.job / 'adoption/adoption.json').read_bytes()
        bundle = (self.job / 'bundle.json').read_bytes()
        old_assets = self.registry()['assets']
        recorded = self.record_later_uses()
        self.assertFalse(recorded['assetRevisionsChanged'])
        self.assertEqual(self.registry()['assets'], old_assets)
        self.verify_execution()
        reused = self.run_execution()
        self.assertTrue(reused['reused'])
        self.assertEqual(reused['stepsRun'], [])
        self.assertEqual((self.job / 'adoption/adoption.json').read_bytes(), receipt)
        self.assertEqual((self.job / 'bundle.json').read_bytes(), bundle)
        self.asset_change(revise=True)
        self.assertIn('inputs changed', self.verify_execution(ok=False))

    def test_real_cli_adopts_verifies_and_reuses_without_repeating_steps(self):
        first = self.run_execution()
        self.assertEqual(first['stepsRun'], ['sync', 'check', 'export', 'verify'])
        self.assertFalse(first['reused'])
        self.assertEqual(self.verify_execution()['contentReview'], 'NOT_PROVEN')
        tracked = {p: p.read_bytes() for p in (self.job / 'adoption').rglob('*')
                   if p.is_file() and p.name != '.state.lock'}
        second = self.run_execution()
        self.assertTrue(second['reused'])
        self.assertEqual(second['stepsRun'], [])
        self.assertEqual({p: p.read_bytes() for p in tracked}, tracked)
        self.assertFalse((self.job / 'adoption/.lock').exists())

    def test_exact_decimal_timing_survives_local_execution(self):
        before = (self.job / 'bundle.json').read_bytes()
        self.run_execution()
        self.assertEqual((self.job / 'bundle.json').read_bytes(), before)
        self.assertEqual(self.read('bundle.json')['episodes'][0]['draft']['units'][0]['shots'][0]['duration'], 2.5)
        receipt = self.read('adoption/adoption.json')
        preview = (self.job / receipt['preview']).read_text(encoding='utf-8')
        self.assertIn('2.5', preview)
        self.assertIn('0.4', preview)
        self.verify_execution()

    def test_plan_and_selection_must_match_complete_packet_values(self):
        for kind, changed in [('plan', {'fantasyCandidates': [], 'testChange': '另一计划'}),
                              ('selection', {'selectedIds': ['different-selection']})]:
            with self.subTest(kind=kind):
                original = (self.job / (kind + '.json')).read_bytes()
                self.save(kind + '.json', changed)
                packet_name = 'packet-' + kind + '.json'
                self.build_packet(packet_name)
                manifest = self.read('execution-request.json')
                manifest['packet'] = packet_name
                self.save('execution-request.json', manifest)
                error = self.run_execution(ok=False, output='bad-' + kind)
                self.assertIn(kind + ': packet and candidate bundle', error)
                self.assertFalse((self.job / ('bad-' + kind) / 'adoption.json').exists())
                (self.job / (kind + '.json')).write_bytes(original)

    def test_input_and_preview_tampering_preserves_old_receipt_but_rejects_reuse(self):
        self.run_execution()
        receipt_bytes = (self.job / 'adoption/adoption.json').read_bytes()
        original = (self.job / 'plan.json').read_bytes()
        self.save('plan.json', {'fantasyCandidates': [], 'changedAfterAdoption': True})
        self.verify_execution(ok=False)
        self.run_execution(ok=False)
        (self.job / 'plan.json').write_bytes(original)
        preview = self.job / self.read('adoption/adoption.json')['preview']
        preview.write_text(preview.read_text(encoding='utf-8') + '\n被篡改的输出\n', encoding='utf-8')
        self.assertIn('output changed', self.verify_execution(ok=False))
        self.assertIn('output changed', self.run_execution(ok=False))
        self.assertEqual((self.job / 'adoption/adoption.json').read_bytes(), receipt_bytes)

    def test_changed_candidate_requires_new_output_directory(self):
        self.run_execution()
        self.bundle['title'] += '新候选'
        self.save('bundle.json', self.bundle)
        self.assertIn('inputs changed', self.run_execution(ok=False))
        self.assertIn('adoption inputs changed', self.verify_execution(ok=False))

    def test_existing_input_directory_is_rejected_before_report_can_overwrite_source(self):
        protected = self.save('input-files/check.json', self.bundle['episodes'][0]['source'])
        original = protected.read_bytes()
        self.run_execution(ok=False, output='input-files')
        self.assertEqual(protected.read_bytes(), original)
        self.assertFalse((self.job / 'input-files/execution.json').exists())
        self.assertFalse((self.job / 'input-files/adoption.json').exists())

    def test_reserved_adoption_and_export_paths_cannot_overwrite_candidate_inputs(self):
        original_request = self.read('execution-request.json')
        for name, tail in [('receipt-conflict', 'adoption.json'),
                           ('preview-conflict', 'export-0/preview.md'),
                           ('catalog-conflict', 'export-0/preview.md.assets.json')]:
            with self.subTest(path=tail):
                relative = name + '/' + tail
                protected = self.save(relative, self.bundle)
                original = protected.read_bytes()
                self.save('execution-request.json', {**original_request, 'bundle': relative})
                self.run_execution(ok=False, output=name)
                self.assertEqual(protected.read_bytes(), original)
                self.assertFalse((self.job / name / 'execution.json').exists())
                self.assertFalse((self.job / name / 'consumption.json').exists())

    def test_interruption_after_check_resumes_without_repeating_sync_or_check(self):
        original_node = execution.node
        def interrupted(script, args):
            if script == 'storyboard.mjs' and args[0] == 'export':
                raise RuntimeError('simulated process interruption before export')
            return original_node(script, args)
        with patch.object(execution, 'node', side_effect=interrupted):
            with self.assertRaisesRegex(RuntimeError, 'interruption'):
                execution.run(self.job, 'execution-request.json', 'adoption')
        state = self.read('adoption/execution.json')
        self.assertEqual(set(state['steps']), {'sync', 'check'})
        unchanged = {name: (self.job / 'adoption' / name).read_bytes()
                     for name in ('sync.json', 'check.json', 'consumption.json')}
        self.assertFalse((self.job / 'adoption/adoption.json').exists())
        resumed = self.run_execution()
        self.assertEqual(resumed['stepsRun'], ['export', 'verify'])
        self.assertEqual({name: (self.job / 'adoption' / name).read_bytes() for name in unchanged}, unchanged)
        self.verify_execution()

    def test_partial_dual_file_export_is_preserved_and_new_attempt_is_used(self):
        original_node = execution.node
        def interrupted(script, args):
            if script == 'storyboard.mjs' and args[0] == 'export':
                preview = Path(args[args.index('--output') + 1])
                preview.parent.mkdir(parents=True, exist_ok=True)
                preview.write_text('partial bytes retained for inspection', encoding='utf-8')
                raise RuntimeError('simulated interruption between dual export files')
            return original_node(script, args)
        with patch.object(execution, 'node', side_effect=interrupted):
            with self.assertRaisesRegex(RuntimeError, 'interruption'):
                execution.run(self.job, 'execution-request.json', 'adoption')
        partial = self.job / 'adoption/export-0/preview.md'
        before = partial.read_bytes()
        resumed = self.run_execution()
        self.assertEqual(resumed['stepsRun'], ['export', 'verify'])
        self.assertEqual(partial.read_bytes(), before)
        self.assertFalse(Path(str(partial) + '.assets.json').exists())
        self.assertEqual(self.read('adoption/adoption.json')['preview'], 'adoption/export-1/preview.md')
        self.verify_execution()

    def test_consumption_written_before_journal_is_recovered_without_overwrite(self):
        original_node = execution.node
        def interrupted(script, args):
            result = original_node(script, args)
            if script == 'assets.mjs' and args[0] == 'sync':
                raise RuntimeError('simulated interruption after consumption write')
            return result
        with patch.object(execution, 'node', side_effect=interrupted):
            with self.assertRaisesRegex(RuntimeError, 'interruption'):
                execution.run(self.job, 'execution-request.json', 'adoption')
        self.assertEqual(self.read('adoption/execution.json')['steps'], {})
        consumption = self.job / 'adoption/consumption.json'
        original_bytes = consumption.read_bytes()
        resumed = self.run_execution()
        self.assertEqual(resumed['stepsRun'], ['sync', 'check', 'export', 'verify'])
        self.assertEqual(consumption.read_bytes(), original_bytes)
        self.verify_execution()

    def test_both_export_files_written_before_journal_are_verified_and_reused(self):
        original_node = execution.node
        def interrupted(script, args):
            result = original_node(script, args)
            if script == 'storyboard.mjs' and args[0] == 'export':
                raise RuntimeError('simulated interruption after both export writes')
            return result
        with patch.object(execution, 'node', side_effect=interrupted):
            with self.assertRaisesRegex(RuntimeError, 'interruption'):
                execution.run(self.job, 'execution-request.json', 'adoption')
        self.assertEqual(set(self.read('adoption/execution.json')['steps']), {'sync', 'check'})
        output = self.job / 'adoption/export-0/preview.md'
        original_bytes = {p: p.read_bytes() for p in (output, Path(str(output) + '.assets.json'))}
        resumed = self.run_execution()
        self.assertEqual(resumed['stepsRun'], ['export', 'verify'])
        self.assertEqual({p: p.read_bytes() for p in original_bytes}, original_bytes)
        self.assertEqual(self.read('adoption/adoption.json')['preview'], 'adoption/export-0/preview.md')
        self.assertFalse((self.job / 'adoption/export-1').exists())
        self.verify_execution()

    def test_unrelated_registry_addition_reuses_receipt_related_revision_invalidates_it(self):
        self.run_execution()
        receipt = (self.job / 'adoption/adoption.json').read_bytes()
        self.asset_change()
        self.assertEqual(self.run_execution()['stepsRun'], [])
        self.verify_execution()
        self.assertEqual((self.job / 'adoption/adoption.json').read_bytes(), receipt)
        self.asset_change(revise=True)
        self.assertIn('inputs changed', self.verify_execution(ok=False))
        self.run_execution(ok=False)

    def test_completed_adoption_survives_skill_maintenance_but_not_changed_job_inputs(self):
        isolated_skill = self.root / 'isolated-skill'
        shutil.copytree(BASE, isolated_skill, ignore=shutil.ignore_patterns('__pycache__', 'tests'))
        self.scripts = isolated_skill / 'scripts'
        self.run_execution()
        receipt = (self.job / 'adoption/adoption.json').read_bytes()
        (isolated_skill / 'VERSION').write_text('20990101-maintenance-test\n', encoding='utf-8')
        (self.scripts / 'maintenance_tool.py').write_text('# unrelated later maintenance\n', encoding='utf-8')
        self.verify_execution()
        reused = self.run_execution()
        self.assertTrue(reused['reused'])
        self.assertEqual(reused['stepsRun'], [])
        self.cli('job_packet.py', 'verify', '--workdir', self.job, '--packet', 'packet.json', ok=False)
        self.assertEqual((self.job / 'adoption/adoption.json').read_bytes(), receipt)
        source = self.read('source.json')
        source['sourceRows'][0]['text'] += '未授权的新源句。'
        self.save('source.json', source)
        self.verify_execution(ok=False)
        self.run_execution(ok=False)
        self.assertEqual((self.job / 'adoption/adoption.json').read_bytes(), receipt)

    def test_workspace_handoff_and_complete_bind_the_real_adoption_receipt(self):
        self.cli('workspace.py', 'claim', '--workdir', self.job, '--group', 1, '--owner', '/root/author', '--full')
        self.save('handoff.txt', '完整作者候选已保存；由整编者接任本集，无中断或恢复。')
        handoff_args = ('handoff', '--workdir', self.job, '--episode', 1, '--owner', '/root/author',
                        '--to', '/root/finisher', '--candidate', 'draft.json', '--evidence', 'handoff.txt', '--full')
        first = self.cli('workspace.py', *handoff_args)
        repeated = self.cli('workspace.py', *handoff_args)
        self.assertFalse(first['reusedHandoff'])
        self.assertTrue(repeated['reusedHandoff'])
        state = self.read('state.json')
        self.assertEqual(state['episodes']['1']['status'], 'in_flight')
        self.assertEqual(state['episodes']['1']['owner'], '/root/finisher')
        self.assertEqual(len(state['episodes']['1']['handoffs']), 1)
        self.run_execution()
        evidence = {'version': 'series-episode-evidence-v1', 'episode': 1,
                    'sourceHash': self.source_hash, 'sourceIndexHash': state['sourceIndexHash'],
                    'scenes': ['1-1', '1-2'], 'reviewMode': 'independent',
                    'artifacts': {'intake': 'source.json', 'assets': 'registry/v000001.json',
                                  'direction': 'bundle.json', 'delivery': 'adoption/export-0/preview.md'},
                    'adopted': 'bundle.json', 'adoptionReceipt': 'adoption/adoption.json'}
        self.save('completion.json', evidence)
        completed = self.cli('workspace.py', 'complete', '--workdir', self.job, '--episode', 1,
                             '--owner', '/root/finisher', '--evidence', 'completion.json', '--full')
        self.assertEqual(completed['episodes']['1']['status'], 'complete')
        self.assertTrue(completed['readyForDelivery'])
        saved = self.read('state.json')['episodes']['1']['completion']['adoptionReceipt']
        self.assertEqual(saved['path'], 'adoption/adoption.json')
        self.verify_execution()
        self.record_later_uses()
        after_usage = self.cli('workspace.py', 'status', '--workdir', self.job, '--full')
        self.assertTrue(after_usage['readyForDelivery'])
        self.assertEqual(after_usage['episodes']['1']['status'], 'complete')
        self.assertEqual(self.run_execution()['stepsRun'], [])
        # A synthetic scaled status read reuses real local adoption evidence.
        # It does not claim that 25 different creative episodes were produced.
        status_state = self.read('state.json')
        status_state['scope'] = status_state['sourceOrder'] = list(range(1, 26))
        status_state['episodes'] = {str(n): copy.deepcopy(status_state['episodes']['1']) for n in range(1, 26)}
        with patch.object(execution, 'node', side_effect=AssertionError('status must not spawn a Node verifier')) as node_calls:
            scaled = workspace.view(self.job, status_state)
            self.assertTrue(scaled['textComplete'])
            self.assertEqual(len(scaled['episodes']), 25)
            self.assertEqual(node_calls.call_count, 0)
        self.asset_change(revise=True)
        status = self.cli('workspace.py', 'status', '--workdir', self.job, '--full')
        self.assertFalse(status['readyForDelivery'])


if __name__ == '__main__':
    unittest.main()
