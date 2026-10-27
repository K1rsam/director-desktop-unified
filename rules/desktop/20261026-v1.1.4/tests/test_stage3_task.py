"""Observable Stage 3 dispatch boundaries; no model or production writes."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from copy import deepcopy


SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location('stage3_task', SCRIPTS / 'stage3-task.py')
task = importlib.util.module_from_spec(spec)
spec.loader.exec_module(task)
import workspace
import season_scope


class Stage3TaskTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.job, self.skill = self.root / 'job', self.root / 'skill'
        self.job.mkdir()
        self.skill.mkdir()
        self.source = {'key': 'E1', 'sourceProtocol': 'source-standard-v4',
                       'sourceRows': [{'id': 'R1', 'text': '甲：甲乙丙丁'}],
                       'contextRows': [{'id': 'OLD', 'text': '旧事只读'}],
                       'dialogue': [{'id': 'D1', 'rowId': 'R1', 'quote': '甲乙丙丁',
                                     'speaker': '甲', 'channel': '对白', 'kind': 'speech'}]}
        self.draft = {'units': [{'id': 'U1', 'shots': [
            {'id': 'S1', 'duration': 3, 'sourceRefs': ['R1'],
             'dialogueParts': [{'lineId': 'D1', 'text': '甲乙', 'voiceStart': 1, 'voiceEnd': 3}]},
            {'id': 'S2', 'duration': 2, 'sourceRefs': ['R1'],
             'dialogueParts': [{'lineId': 'D1', 'text': '丙丁', 'voiceStart': 0, 'voiceEnd': 1}]}]}]}
        self.request = {'version': task.REQUEST_VERSION, 'episode': 1, 'role': 'author',
                        'task': '完成本集完整主创。',
                        'fantasy': {'mode': 'off', 'selected': False, 'sourceRequired': False}, 'inputs': []}
        self.add('source', 'production', 'source.json', self.source)
        self.add('direction', 'support', 'direction.json', {'ratio': '16:9', 'fantasy': 'off'})
        self.baseline = {
            'version': 'series-visual-baseline-v1', 'id': 'fixture-style', 'revision': 1,
            'status': 'adopted',
            'visualStyle': '自然比例的二维人物。水彩平涂着色。纸纹与布料柔哑反射。冷灰色彩和侧向软光。16:9。局部图解按镜头起止回接现实。',
            'coverage': {'form': '自然比例的二维人物', 'rendering': '水彩平涂着色',
                         'materials': '纸纹与布料柔哑反射', 'colorLighting': '冷灰色彩和侧向软光',
                         'aspectRatio': '16:9', 'localExceptions': '局部图解按镜头起止回接现实'},
            'adoptionBasis': 'test_fixture：沿用已确认完整美术说明。'}
        self.add('visual-baseline', 'support', 'baseline.json', self.baseline)
        role_file = self.skill / task.ROLE_FILE
        role_file.parent.mkdir(parents=True)
        role_file.write_text('\n'.join('## ' + r + '\n' + r + '执行正文。\n'
                                      for r in ['common'] + sorted(task.ROLES)), encoding='utf-8')
        paths = {'references/creative/references/common-contract.md'}
        names = {n for methods in task.METHODS.values() for n in methods} | {'fantasy-expression-methods'}
        paths |= {'references/creative/skills/' + n + '/SKILL.md' for n in names}
        for path in paths:
            dest = self.skill / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text('具体方法正文：' + path, encoding='utf-8')

    def add(self, kind, purpose, path, value, pointer=None):
        raw = task.cp.canonical(value)
        (self.job / path).write_bytes(raw)
        item = {'kind': kind, 'purpose': purpose, 'path': path, 'sha256': task.cp.sha256(raw)}
        if pointer is not None:
            item['pointer'] = pointer
        self.request['inputs'].append(item)
        return item

    def finishing(self, role='finisher'):
        self.request['role'] = role
        self.add('draft', 'production', 'draft.json', self.draft)
        self.add('plan', 'support', 'plan.json', {'events': ['event1']})
        self.add('selection', 'support', 'selection.json', {'selected': []})

    def build(self, max_bytes=524288):
        (self.job / 'request.json').write_bytes(task.cp.canonical(self.request))
        return task.build(self.job, 'request.json', 'tasks/job.json', 'tasks/dispatch.md', max_bytes, self.skill)

    def verify(self):
        return task.verify(self.job, 'tasks/job.json', self.skill)

    def replace_baseline(self):
        raw = task.cp.canonical(self.baseline)
        (self.job / 'baseline.json').write_bytes(raw)
        item = next(i for i in self.request['inputs'] if i['kind'] == 'visual-baseline')
        item['sha256'] = task.cp.sha256(raw)

    def save_output(self, baseline=None, visual_style=None, number=1):
        baseline = self.baseline if baseline is None else baseline
        result = {'version': 'series-storyboard-bundle-v1', 'exportProfile': 'lingmo-v7',
                  'episodes': [{'number': number, 'settings': {'visualBaseline': baseline,
                               'visualStyle': baseline['visualStyle'] if visual_style is None else visual_style},
                               'draft': self.draft}]}
        (self.job / 'output.json').write_bytes(task.cp.canonical(result))
        return result

    def verify_output(self):
        return task.verify_output(self.job, 'tasks/job.json', 'output.json', self.skill)

    def test_verify_output_binds_custom_style_and_preserves_all_bytes(self):
        self.build()
        self.save_output()
        before = (self.job / 'output.json').read_bytes()
        proof = self.verify_output()
        self.assertEqual(proof['verificationMode'], 'visual-baseline-binding-only')
        self.assertEqual(proof['visualBaseline']['id'], self.baseline['id'])
        self.assertEqual(proof['bundle']['sha256'], task.cp.sha256(before))
        self.assertEqual(proof['task']['sha256'], task.cp.sha256((self.job / 'tasks/job.json').read_bytes()))
        self.assertEqual(proof['bundleStructure'], 'NOT_CHECKED')
        self.assertEqual((self.job / 'output.json').read_bytes(), before)

    def test_verify_output_rejects_other_valid_baseline_and_abbreviation(self):
        self.build()
        other = deepcopy(self.baseline)
        other.update(id='another-approved-style', revision=2)
        task.validate_visual_baseline(other)  # B is valid, but not this task's A.
        self.save_output(other)
        with self.assertRaisesRegex(task.cp.PacketError, 'visualBaseline differs'):
            self.verify_output()
        self.save_output(visual_style='二维水彩，16:9')
        with self.assertRaisesRegex(task.cp.PacketError, 'complete task baseline'):
            self.verify_output()

    def test_verify_output_rejects_missing_style_wrong_episode_and_multiple_episodes(self):
        self.build()
        result = self.save_output()
        del result['episodes'][0]['settings']['visualBaseline']
        (self.job / 'output.json').write_bytes(task.cp.canonical(result))
        with self.assertRaisesRegex(task.cp.PacketError, 'visualBaseline differs'):
            self.verify_output()
        self.save_output(number=2)
        with self.assertRaisesRegex(task.cp.PacketError, 'episode differs'):
            self.verify_output()
        result = self.save_output()
        result['episodes'].append(deepcopy(result['episodes'][0]))
        (self.job / 'output.json').write_bytes(task.cp.canonical(result))
        with self.assertRaisesRegex(task.cp.PacketError, 'exactly one'):
            self.verify_output()

    def test_verify_output_rechecks_current_task_before_accepting_bundle(self):
        self.build()
        self.save_output()
        (self.job / 'source.json').write_bytes((self.job / 'source.json').read_bytes() + b' ')
        with self.assertRaisesRegex(task.cp.PacketError, 'sha256'):
            self.verify_output()

    def test_verify_output_cli_returns_narrow_proof(self):
        (self.job / 'request.json').write_bytes(task.cp.canonical(self.request))
        task.build(self.job, 'request.json', 'tasks/job.json', 'tasks/dispatch.md', 524288, SCRIPTS.parent)
        self.save_output()
        result = subprocess.run([sys.executable, str(SCRIPTS / 'stage3-task.py'), 'verify-output',
                                 '--workdir', str(self.job), '--task', 'tasks/job.json', '--bundle', 'output.json'],
                                capture_output=True, text=True, encoding='utf-8', timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        proof = json.loads(result.stdout)
        self.assertEqual(proof['episode'], 1)
        self.assertEqual(proof['quality'], 'NOT_REVIEWED')

    def test_adopted_visual_baseline_is_exact_and_dispatch_keeps_unit_performance(self):
        packet, raw, dispatch = self.build()
        material = next(m for m in packet['materials'] if m['kind'] == 'visual-baseline')
        self.assertEqual(task.selected_value(material), self.baseline)
        self.assertEqual(packet['version'], 'desktop-stage3-task-v2')
        self.assertEqual(packet['visualBaseline']['settingsCheck'], 'BUNDLE_SETTINGS_CHECK_REQUIRED')
        self.assertIn('unitDirection', dispatch.decode())
        self.assertEqual(self.verify()[1], raw)

    def test_new_task_without_baseline_rejected_without_legacy_escape(self):
        self.request['inputs'] = [i for i in self.request['inputs'] if i['kind'] != 'visual-baseline']
        with self.assertRaisesRegex(task.cp.PacketError, 'visual-baseline'):
            self.build()
        self.request['historical'] = True
        with self.assertRaisesRegex(task.cp.PacketError, 'invalid fields'):
            self.build()
        self.assertFalse((self.job / 'tasks/job.json').exists())

    def test_candidate_and_fabricated_coverage_rejected(self):
        self.baseline['status'] = 'candidate'
        self.replace_baseline()
        with self.assertRaisesRegex(ValueError, 'adopted baseline'):
            self.build()
        self.baseline['status'] = 'adopted'
        self.baseline['coverage']['materials'] = '根本不在完整基准中的说明'
        self.replace_baseline()
        with self.assertRaisesRegex(ValueError, 'coverage excerpt'):
            self.build()
        self.baseline['coverage'] = dict.fromkeys(self.baseline['coverage'], '16:9')
        self.replace_baseline()
        with self.assertRaisesRegex(ValueError, 'repeated label'):
            self.build()

    def test_json_pointer_baseline_and_duplicate_rejection(self):
        item = next(i for i in self.request['inputs'] if i['kind'] == 'visual-baseline')
        raw = task.cp.canonical({'baseline': self.baseline, 'notes': 'not in role input'})
        (self.job / 'baseline.json').write_bytes(raw)
        item.update(pointer='/baseline', sha256=task.cp.sha256(raw))
        packet, _, _ = self.build()
        material = next(m for m in packet['materials'] if m['kind'] == 'visual-baseline')
        self.assertEqual(material['value'], self.baseline)
        self.assertNotIn('notes', material['value'])
        self.request['inputs'].append(dict(item))
        (self.job / 'request-duplicate.json').write_bytes(task.cp.canonical(self.request))
        with self.assertRaisesRegex(task.cp.PacketError, 'exactly one'):
            task.assemble(self.job, 'request-duplicate.json', 'tasks/other.json', 'tasks/other.md', 524288, self.skill)

    def test_visual_baseline_drift_and_draft_settings_mismatch(self):
        self.finishing()
        self.draft['settings'] = {'visualBaseline': self.baseline, 'visualStyle': '简写成标签'}
        item = next(i for i in self.request['inputs'] if i['kind'] == 'draft')
        raw = task.cp.canonical(self.draft)
        (self.job / 'draft.json').write_bytes(raw)
        item['sha256'] = task.cp.sha256(raw)
        with self.assertRaisesRegex(task.cp.PacketError, 'settings differ'):
            self.build()
        self.draft['settings']['visualStyle'] = self.baseline['visualStyle']
        raw = task.cp.canonical(self.draft)
        (self.job / 'draft.json').write_bytes(raw)
        item['sha256'] = task.cp.sha256(raw)
        packet, _, _ = self.build()
        self.assertEqual(packet['visualBaseline']['settingsCheck'], 'DRAFT_SETTINGS_MATCHED')
        (self.job / 'baseline.json').write_bytes((self.job / 'baseline.json').read_bytes() + b' ')
        with self.assertRaisesRegex(task.cp.PacketError, 'sha256'):
            self.verify()

    def test_frozen_legacy_task_remains_readable_but_cannot_build_new(self):
        # A synthetic old task, with old frozen rule/dispatch text. No live Skill
        # or adopted creative output is used or represented by this fixture.
        packet, _, dispatch = self.build()
        packet['version'] = task.LEGACY_TASK_VERSION
        packet.pop('visualBaseline')
        self.request['inputs'] = [i for i in self.request['inputs'] if i['kind'] != 'visual-baseline']
        request_raw = task.cp.canonical(self.request)
        (self.job / 'request.json').write_bytes(request_raw)
        packet['request'] = self.request
        packet['requestOrigin']['sha256'] = task.cp.sha256(request_raw)
        packet['materials'] = [i for i in packet['materials'] if i['kind'] != 'visual-baseline']
        packet['kindBytes'].pop('visual-baseline')
        packet['taskSha256'] = task.cp.sha256(task.cp.canonical({k: v for k, v in packet.items() if k != 'taskSha256'}))
        raw = task.cp.canonical(packet)
        (self.job / 'tasks/job.json').write_bytes(raw)
        (self.skill / task.ROLE_FILE).write_text('已升级，旧规则正文仍保存在包中。', encoding='utf-8')
        self.assertEqual(self.verify()[1], raw)
        self.save_output()
        with self.assertRaisesRegex(task.cp.PacketError, 'legacy frozen tasks'):
            self.verify_output()
        with self.assertRaisesRegex(task.cp.PacketError, 'visual-baseline'):
            task.build(self.job, 'request.json', 'tasks/new.json', 'tasks/new.md', 524288, self.skill)
        (self.job / 'tasks/dispatch.md').write_bytes(dispatch + b'changed')
        with self.assertRaisesRegex(task.cp.PacketError, 'dispatch'):
            self.verify()

    def replace_source(self):
        raw = task.cp.canonical(self.source)
        (self.job / 'source.json').write_bytes(raw)
        self.request['inputs'][0]['sha256'] = task.cp.sha256(raw)

    def lock_source(self, source_hash='a' * 64):
        self.source.pop('sourceProtocol', None)
        self.source['contextRows'][0]['text'] = '△旧事只读'
        self.source['dialogue'][0]['channel'] = '现场对白'
        self.replace_source()
        lock = {'version': 'series-source-lock-v1', 'sourceHash': source_hash,
                'episodes': [{'number': 1, 'source': deepcopy(self.source)}]}
        raw = json.dumps(lock, ensure_ascii=False, indent=2).encode('utf-8')
        (self.job / 'source-lock.json').write_bytes(raw)
        self.request['sourceLock'] = {'path': 'source-lock.json', 'sha256': task.cp.sha256(raw)}
        return lock

    def season_workspace(self):
        original = self.root / 'original.txt'
        original.write_text('test_fixture: three original episodes\n', encoding='utf-8')
        extracted = self.root / 'extracted'
        manifest = workspace.extract(original, extracted)
        for path in extracted.iterdir():
            shutil.copy2(path, self.job / path.name)
        reading = [{'number': n, 'sceneIds': [f'{n}-1']} for n in (1, 2, 3)]
        scope = {'version': 'series-season-scope-v1', 'id': 's1', 'revision': 1,
                 'seasonNumber': 1, 'status': 'locked', 'sourceHash': manifest['sha256'],
                 'readingScope': reading, 'episodes': reading[:2],
                 'target': {'minSeconds': 7200, 'maxSeconds': 9000, 'planningSeconds': 8100},
                 'ending': {'episode': 2, 'sceneId': '2-1', 'sourceRef': 'source-body.txt:L1',
                            'closure': '原生闭合', 'hook': '源文后续悬念',
                            'estimate': {'minSeconds': 7600, 'maxSeconds': 8500}},
                 'confirmation': {'kind': 'delegated', 'basis': 'test_fixture 授权'}}
        index = {'version': 'series-source-v1', 'sourceHash': manifest['sha256'],
                 'episodes': [{'number': n, 'scenes': [{'id': f'{n}-1'}]} for n in (1, 2, 3)],
                 'productionScope': scope}
        index_path = self.root / 'index.json'
        index_path.write_bytes(task.cp.canonical(index))
        workspace.init(self.job, index_path, 1, None)
        return index

    def test_author_without_plan_is_one_task_with_planner_rules_and_readable_methods(self):
        packet, raw, dispatch = self.build()
        self.assertEqual(packet['role'], 'author')
        self.assertEqual(packet['workerRead'], 'NOT_ATTESTED')
        self.assertEqual(packet['quality'], 'NOT_REVIEWED')
        paths = [r['path'] for r in packet['rules']]
        self.assertTrue(any('episode-emotion-planner' in p for p in paths))
        self.assertFalse(any('fantasy-expression-methods' in p for p in paths))
        self.assertTrue(all(r.get('text') for r in packet['rules']))
        self.assertIn('同一主创上下文', dispatch.decode())
        self.assertIn('materials 和 rules', dispatch.decode())
        self.assertIn('rules[2].text', dispatch.decode())
        self.assertNotIn('具体方法正文：', dispatch.decode())  # full methods occur once in the packet
        self.assertEqual(packet['scope']['productionRowIds'], ['R1'])
        self.assertEqual(packet['scope']['contextRowIds'], ['OLD'])
        self.assertEqual(self.verify()[1], raw)

    def test_existing_plan_not_replaced_and_no_default_second_planner(self):
        self.add('plan', 'support', 'plan.json', {'events': ['keep']})
        self.add('selection', 'support', 'selection.json', {'selected': []})
        packet, _, _ = self.build()
        self.assertFalse(any('episode-emotion-planner' in r['path'] for r in packet['rules']))
        self.assertEqual(next(m for m in packet['materials'] if m['kind'] == 'plan')['value'], '{"events":["keep"]}')

    def test_finisher_embeds_anchored_signals_before_any_candidate(self):
        self.finishing()
        packet, _, _ = self.build()
        self.assertFalse((self.job / 'candidate.json').exists())
        signals = packet['editingSignals']
        group = signals['source_signals']['same_line_cross_shot'][0]
        self.assertEqual(group['source_anchor']['quote'], '甲乙丙丁')
        self.assertEqual(group['source_anchor']['sourceRows'][0]['id'], 'R1')
        self.assertEqual(group['joins'][0]['relation'], 'CONTIGUOUS')
        self.assertEqual(group['rate_max_over_min'], 2)
        self.assertEqual(signals['timeline']['unit_tails'][0]['seconds'], 1)
        self.assertEqual(packet['role'], 'finisher')
        self.verify()

    def test_real_roles_require_existing_complete_materials(self):
        for role in ('finisher', 'reviewer', 'refiner'):
            with self.subTest(role=role):
                self.request['role'] = role
                with self.assertRaisesRegex(task.cp.PacketError, 'missing role materials'):
                    self.build()
                self.assertFalse((self.job / 'tasks/job.json').exists())
        self.request['role'] = 'planner'
        self.assertEqual(self.build()[0]['role'], 'planner')

    def test_reviewer_and_refiner_keep_truthful_role_and_draft(self):
        self.finishing('reviewer')
        packet, _, dispatch = self.build()
        self.assertEqual(packet['role'], 'reviewer')
        self.assertIn('本岗只审阅', dispatch.decode())
        self.request['role'] = 'refiner'
        (self.job / 'request2.json').write_bytes(task.cp.canonical(self.request))
        second, _, _ = task.build(self.job, 'request2.json', 'tasks/two.json', 'tasks/two.md', 524288, self.skill)
        self.assertEqual(second['role'], 'refiner')
        self.assertTrue(any('director-refinement' in r['path'] for r in second['rules']))

    def test_rules_changed_after_freeze_rejected_even_when_materials_unchanged(self):
        self.build()
        rules = self.skill / task.ROLE_FILE
        rules.write_text(rules.read_text(encoding='utf-8') + '\n新增规则。', encoding='utf-8')
        with self.assertRaisesRegex(task.cp.PacketError, 'drift'):
            self.verify()

    def test_source_drift_and_dispatch_tamper_detected(self):
        self.build()
        original = (self.job / 'source.json').read_bytes()
        (self.job / 'source.json').write_bytes(original + b' ')
        with self.assertRaisesRegex(task.cp.PacketError, 'sha256'):
            self.verify()
        (self.job / 'source.json').write_bytes(original)
        (self.job / 'tasks/dispatch.md').write_text('tampered', encoding='utf-8')
        with self.assertRaisesRegex(task.cp.PacketError, 'dispatch'):
            self.verify()

    def test_over_budget_never_truncates_or_writes_either_output(self):
        with self.assertRaisesRegex(task.cp.PacketError, 'no truncation'):
            self.build(max_bytes=200)
        self.assertFalse((self.job / 'tasks/job.json').exists())
        self.assertFalse((self.job / 'tasks/dispatch.md').exists())
        packet, raw, dispatch = self.build()
        self.assertLessEqual(len(raw) + len(dispatch), packet['maxBytes'])

    def test_context_is_not_promoted_into_production_and_overlap_rejected(self):
        self.add('source', 'context', 'context.json', {'sourceRows': [{'id': 'C1', 'text': '前情'}]})
        self.finishing()
        self.draft['units'][0]['shots'][0]['sourceRefs'] = ['C1']
        item = next(i for i in self.request['inputs'] if i['kind'] == 'draft')
        raw = task.cp.canonical(self.draft)
        (self.job / 'draft.json').write_bytes(raw)
        item['sha256'] = task.cp.sha256(raw)
        with self.assertRaisesRegex(task.cp.PacketError, 'sourceRefs'):
            self.build()

    def test_same_projection_cannot_be_disguised_as_another_purpose(self):
        duplicate = dict(self.request['inputs'][0], purpose='context')
        self.request['inputs'].append(duplicate)
        with self.assertRaisesRegex(task.cp.PacketError, 'duplicate path/pointer'):
            self.build()

    def test_full_source_and_draft_cannot_be_replaced_by_partial_subtrees(self):
        self.finishing()
        item = next(i for i in self.request['inputs'] if i['kind'] == 'draft')
        item['pointer'] = '/units/0'
        with self.assertRaisesRegex(task.cp.PacketError, 'complete draft'):
            self.build()
        del item['pointer']
        self.request['inputs'][0]['pointer'] = '/sourceRows/0'
        with self.assertRaisesRegex(task.cp.PacketError, 'complete normalized'):
            self.build()

    def test_projection_does_not_leak_neighbor_episode_or_history(self):
        item = self.request['inputs'][0]
        raw = task.cp.canonical({'episodes': [self.source], 'history': 'UNRELATED_HISTORY_DO_NOT_LOAD'})
        (self.job / 'source.json').write_bytes(raw)
        item.update(sha256=task.cp.sha256(raw), pointer='/episodes/0')
        _, packet, dispatch = self.build()
        self.assertNotIn(b'UNRELATED_HISTORY_DO_NOT_LOAD', packet + dispatch)
        self.verify()

    def test_source_required_fantasy_loads_method_even_with_extra_fantasy_off(self):
        self.request['fantasy']['sourceRequired'] = True
        packet, _, _ = self.build()
        self.assertTrue(any('fantasy-expression-methods' in r['path'] for r in packet['rules']))

    def test_exact_prose_context_keeps_purpose_without_inventing_source_rows(self):
        raw = '前集只读；不得重复拍摄。\n'.encode('utf-8')
        (self.job / 'prior.txt').write_bytes(raw)
        self.request['inputs'].append({'kind': 'source', 'purpose': 'context', 'path': 'prior.txt',
                                       'sha256': task.cp.sha256(raw)})
        packet, _, _ = self.build()
        context = next(m for m in packet['materials'] if m['purpose'] == 'context')
        self.assertEqual(context['value'].encode('utf-8'), raw)
        self.assertEqual(packet['scope']['productionRowIds'], ['R1'])
        self.assertEqual(packet['scope']['contextRowIds'], ['OLD'])

    def test_context_plan_is_not_current_plan_or_fantasy_authorization(self):
        self.finishing()
        next(i for i in self.request['inputs'] if i['kind'] == 'plan')['purpose'] = 'context'
        with self.assertRaisesRegex(task.cp.PacketError, 'missing role materials: plan'):
            self.build()

    def test_sound_event_signal_retains_source_anchor_but_no_speech_rate(self):
        self.finishing()
        for shot in self.draft['units'][0]['shots']:
            shot['dialogueParts'][0]['presentation'] = '声音事件'
        raw = task.cp.canonical(self.draft)
        (self.job / 'draft.json').write_bytes(raw)
        next(i for i in self.request['inputs'] if i['kind'] == 'draft')['sha256'] = task.cp.sha256(raw)
        packet, _, _ = self.build()
        signals = packet['editingSignals']
        self.assertEqual(signals['timeline']['voice_seconds'], 0)
        self.assertEqual(len(signals['timeline']['sound_event_windows']), 2)
        group = signals['source_signals']['same_line_cross_shot'][0]
        self.assertIsNone(group['rate_min'])
        self.assertEqual(group['source_anchor']['quote'], '甲乙丙丁')

    def test_existing_dispatch_is_preserved_without_partial_task(self):
        (self.job / 'tasks').mkdir()
        (self.job / 'tasks/dispatch.md').write_bytes(b'KEEP')
        with self.assertRaisesRegex(task.cp.PacketError, 'exists'):
            self.build()
        self.assertEqual((self.job / 'tasks/dispatch.md').read_bytes(), b'KEEP')
        self.assertFalse((self.job / 'tasks/job.json').exists())

    def test_actual_stage3_cli_blocks_future_episode_with_valid_input_hashes(self):
        self.season_workspace()
        self.request['episode'] = 3
        (self.job / 'request.json').write_bytes(task.cp.canonical(self.request))
        result = subprocess.run([sys.executable, '-X', 'utf8', str(SCRIPTS / 'stage3-task.py'),
                                 'build', '--workdir', str(self.job), '--manifest', 'request.json',
                                 '--output', 'tasks/job.json', '--dispatch', 'tasks/dispatch.md'],
                                capture_output=True, text=True, encoding='utf-8')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('outside locked season', result.stderr)
        self.assertFalse((self.job / 'tasks/job.json').exists())
        self.assertFalse((self.job / 'tasks/dispatch.md').exists())

    def test_selected_season_task_binds_scope_and_independent_source_lock(self):
        index = self.season_workspace()
        self.lock_source(index['sourceHash'])
        packet, raw, _ = self.build()
        self.assertEqual(packet['scopeHash'], workspace.load_state(self.job)['scopeHash'])
        self.assertEqual(packet['sourceFidelity']['mode'], 'locked-production-source-v1')
        self.assertEqual(self.verify()[1], raw)
        state_path = self.job / 'state.json'
        state = json.loads(state_path.read_text(encoding='utf-8'))
        state['scope'].append(3)
        state_path.write_bytes(task.cp.canonical(state))
        with self.assertRaisesRegex(task.cp.PacketError, 'workspace scope'):
            self.verify()

    def test_season_task_cannot_omit_independent_source_lock(self):
        self.season_workspace()
        with self.assertRaisesRegex(task.cp.PacketError, 'requires independent sourceLock'):
            self.build()
        self.assertFalse((self.job / 'tasks/job.json').exists())

    def test_season_index_without_workspace_cannot_be_used_as_legacy_task(self):
        self.season_workspace()
        (self.job / 'state.json').unlink()
        with self.assertRaisesRegex(task.cp.PacketError, 'state is missing'):
            self.build()

    def test_sequel_stage3_task_requires_actual_closed_handoff(self):
        index = self.season_workspace()
        self.lock_source(index['sourceHash'])
        scope = index['productionScope']
        scope.update(id='s2', seasonNumber=2,
                     continuation={'previousScopeHash': 'f' * 64, 'previousSeasonId': 's1', 'nextEpisode': 1})
        index_path = self.root / 'index.json'
        index_path.write_bytes(task.cp.canonical(index))
        projected = season_scope.project_index(index_path)
        (self.job / 'source-index.json').write_bytes(task.cp.canonical(projected))
        state_path = self.job / 'state.json'
        state = json.loads(state_path.read_text(encoding='utf-8'))
        state.update(productionScope=projected['productionScope'], scopeHash=projected['scopeHash'],
                     sourceIndexHash=workspace.sha(self.job / 'source-index.json'))
        state_path.write_bytes(task.cp.canonical(state))
        with self.assertRaisesRegex(task.cp.PacketError, 'previous-handoff|closed|闭季'):
            self.build()
        self.assertFalse((self.job / 'tasks/job.json').exists())

    def test_source_lock_rejects_joint_source_and_dialogue_rewrite(self):
        self.lock_source()
        self.source['sourceRows'][0]['text'] = '甲：修改过的话'
        self.source['dialogue'][0]['quote'] = '修改过的话'
        self.replace_source()
        with self.assertRaisesRegex(task.cp.PacketError, 'sourceLock'):
            self.build()
        self.assertFalse((self.job / 'tasks/job.json').exists())

    def test_lock_file_drift_rejected_on_task_verify(self):
        lock = self.lock_source()
        packet, _, _ = self.build()
        expected = task.cp.sha256(json.dumps(lock, ensure_ascii=False, separators=(',', ':')).encode('utf-8'))
        self.assertEqual(packet['sourceFidelity']['sourceLockHash'], expected)
        with (self.job / 'source-lock.json').open('ab') as stream:
            stream.write(b' ')
        with self.assertRaisesRegex(task.cp.PacketError, 'sourceLock file sha256'):
            self.verify()

    def test_compact_standard_source_uses_frozen_receipt_without_copying_it_to_task(self):
        episode_text = '第1集\n1-1 日 内 门厅\n人物：守门人\n连续性备注：面罩不摘。\n守门人（OS）：只是风。\n字幕：次日\n'
        program = "import{prepareSource}from './scripts/prepare-source.mjs';process.stdout.write(JSON.stringify(prepareSource(JSON.parse(process.argv[1]),'E1',[])))"
        result = subprocess.run([os.environ.get('SERIES_NODE', 'node'), '--input-type=module', '-e',
                                 program, json.dumps(episode_text)], cwd=SCRIPTS.parent,
                                capture_output=True, text=True, encoding='utf-8')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.source = json.loads(result.stdout)
        raw = task.cp.canonical({'version': 'series-source-lock-v1', 'sourceHash': 'a' * 64,
                                 'episodes': [{'number': 1, 'source': deepcopy(self.source)}]})
        (self.job / 'source-lock.json').write_bytes(raw)
        self.request['sourceLock'] = {'path': 'source-lock.json', 'sha256': task.cp.sha256(raw)}
        del self.source['standardPackage']
        del self.source['intakeEvidence']
        self.source['contextRows'] = []
        self.replace_source()
        packet, raw, _ = self.build()
        self.assertEqual(packet['sourceFidelity']['mode'], 'locked-production-source-v1')
        material = json.loads(packet['materials'][0]['value'])
        self.assertNotIn('standardPackage', material)
        self.assertNotIn('intakeEvidence', material)
        self.assertEqual(material['contextRows'], [])
        self.assertEqual(self.verify()[1], raw)


if __name__ == '__main__':
    unittest.main()
