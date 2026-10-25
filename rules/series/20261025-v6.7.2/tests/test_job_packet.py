"""Behavior tests for exact role input compilation, drift and v7 rules."""
import copy
from decimal import Decimal
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE / 'scripts'))
import job_packet as jp
import workspace


class JobPacketTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.job = self.root / 'job'
        self.job.mkdir()
        self.skill = self.root / 'skill'
        paths = set(jp.BASE_RULES + list(jp.ROLE_RULES.values()) + jp.AUTHOR_RULES + ['VERSION', 'scripts/job_packet.py'])
        paths.update('references/creative/skills/' + method + '/SKILL.md' for method in jp.METHODS)
        paths.update('references/' + name + '.md' for name in jp.OPTIONAL_CONTRACTS)
        for path in paths:
            destination = self.skill / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(BASE / path, destination)
        self.original = '第1集\r\n甲：“不要改字，1.250000001秒。”\r\n末尾空格  \n'
        self.request = {'version': jp.REQUEST_VERSION, 'episode': 1, 'role': 'author',
                        'review': {'path': 'review.md'}, 'inputs': []}
        self.write('review.md', 'test_fixture: 已读本集完整原文及所列完整采用设计，依赖范围由主控审定。')
        for kind, value in [('source', self.original), ('direction', '有效方向'),
                            ('state', '当下事实'), ('assets', '完整采用设计'),
                            ('entry', 'PRELIMINARY，未获前集实际尾稿')]:
            self.add(kind, kind + '.txt', value)

    def write(self, path, value):
        target = self.job / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(value.encode('utf-8') if isinstance(value, str) else value)

    def add(self, kind, path, value, pointer=None):
        self.write(path, value)
        item = {'kind': kind, 'path': path}
        if pointer is not None:
            item['pointer'] = pointer
        self.request['inputs'].append(item)
        return item

    def build(self, output='packets/one.json', limit=jp.DEFAULT_MAX_BYTES):
        self.write('request.json', json.dumps(self.request, ensure_ascii=False))
        return jp.build(self.job, 'request.json', output, limit, self.skill)[0]

    def verify(self, output='packets/one.json'):
        return jp.load_verified(self.job, output, self.skill)

    def role_inputs(self, role):
        self.request['role'] = role
        for kind in ['draft', 'plan', 'selection']:
            self.add(kind, kind + '.json', '{"value":"' + kind + '","seconds":1.250000001}', '')

    def test_full_source_is_not_summarized_or_truncated(self):
        self.original *= 300
        self.write('source.txt', self.original)
        packet = self.build()
        source = packet['bindings'][0]
        self.assertEqual(packet['blobs'][source['blob']]['value'].encode('utf-8'), self.original.encode('utf-8'))
        self.assertEqual(source['sha256'], jp.sha256(self.original.encode('utf-8')))
        self.assertEqual(self.verify()['inputFingerprint'], packet['inputFingerprint'])

    def test_equal_content_is_stored_once_with_all_kinds_and_origins(self):
        self.add('entry', 'entry-copy.txt', self.original)
        self.add('state', 'source.txt', self.original)
        packet = self.build()
        matching = [b for b in packet['bindings'] if b['blob'] == packet['bindings'][0]['blob']]
        self.assertEqual(len(matching), 3)
        self.assertEqual({b['kind'] for b in matching}, {'source', 'entry', 'state'})
        self.assertEqual({b['path'] for b in matching}, {'source.txt', 'entry-copy.txt'})
        self.verify()

    def test_same_json_projection_deduplicates_without_losing_decimal_precision(self):
        number = '1.250000001123456789123456789'
        self.add('plan', 'all.json', '{"one":{"seconds":' + number + '},"two":{"seconds":' + number + '}}', '/one')
        self.request['inputs'].append({'kind': 'selection', 'path': 'all.json', 'pointer': '/two'})
        packet = self.build()
        a, b = packet['bindings'][-2:]
        self.assertEqual(a['blob'], b['blob'])
        self.assertNotEqual(a['pointer'], b['pointer'])
        self.assertEqual(packet['blobs'][a['blob']]['value']['seconds'], Decimal(number))
        self.assertIn(number.encode(), (self.job / 'packets/one.json').read_bytes())
        self.verify()

    def test_role_required_material_and_review_record(self):
        original = copy.deepcopy(self.request)
        for kind in jp.REQUIRED:
            self.request = copy.deepcopy(original)
            self.request['inputs'] = [i for i in self.request['inputs'] if i['kind'] != kind]
            with self.assertRaisesRegex(jp.PacketError, 'missing input'):
                self.build()
        for role in ['finisher', 'reviewer']:
            for missing in ['draft', 'plan', 'selection']:
                self.request = copy.deepcopy(original)
                self.role_inputs(role)
                self.request['inputs'] = [i for i in self.request['inputs'] if i['kind'] != missing]
                with self.assertRaisesRegex(jp.PacketError, 'missing input'):
                    self.build()
        self.request = original
        self.write('review.md', ' \n ')
        with self.assertRaisesRegex(jp.PacketError, 'review'):
            self.build()

    def test_v7_effective_view_replaces_legacy_integer_requirement_and_preserves_art(self):
        self.role_inputs('finisher')
        self.request['methods'] = ['director-refinement', 'fantasy-expression-methods']
        packet = self.build()
        for path, changes in jp.V7_REPLACEMENTS.items():
            if path in jp.AUTHOR_RULES:
                continue
            rule = next(r for r in packet['rules'] if r['path'] == path)
            text = packet['blobs'][rule['blob']]['value']
            for name, before, after in changes:
                self.assertNotIn(before, text)
                self.assertIn(after, text)
                self.assertIn(name, rule['adjustments'])
            self.assertEqual(rule['sha256'], jp.sha256((self.skill / path).read_bytes()))
        common = next(r for r in packet['rules'] if r['path'] == jp.COMMON_RULE)
        self.assertIn('源只写“愤怒”不等于只准皱眉握拳', packet['blobs'][common['blob']]['value'])
        self.verify()

    def test_default_four_methods_and_explicit_optional_methods(self):
        packet = self.build()
        paths = [r['path'] for r in packet['rules']]
        for method in jp.CORE_METHODS:
            self.assertEqual(paths.count('references/creative/skills/' + method + '/SKILL.md'), 1)
        self.assertFalse(any('fantasy-expression-methods' in path for path in paths))
        for path in jp.AUTHOR_RULES:
            self.assertIn(path, paths)
        context = next(r for r in packet['rules'] if r['path'] == jp.AUTHOR_RULES[0])
        self.assertIn('required-input-context', context['adjustments'])
        self.assertNotIn('每个单元的镜长合计落在整数秒',
                         ''.join(packet['blobs'][r['blob']]['value'] for r in packet['rules']))
        self.request['methods'] = ['acting-blocking-methods', 'fantasy-expression-methods']
        packet = self.build('packets/two.json')
        paths = [r['path'] for r in packet['rules']]
        self.assertEqual(sum('acting-blocking-methods' in path for path in paths), 1)
        self.assertEqual(sum('fantasy-expression-methods' in path for path in paths), 1)

    def test_reviewer_has_content_review_not_final_whole_season_export_task(self):
        self.role_inputs('reviewer')
        packet = self.build()
        rule = next(r for r in packet['rules'] if r['path'] == jp.ROLE_RULES['reviewer'])
        text = packet['blobs'][rule['blob']]['value']
        self.assertIn('从源到稿', text)
        self.assertIn('从稿到源', text)
        self.assertNotIn('## 编译与交付', text)
        self.assertIn('不代写修稿', packet['task']['role'])
        production = next(r for r in packet['rules'] if r['path'] == 'references/production-contract.md')
        text = packet['blobs'][production['blob']]['value']
        self.assertIn('幻想候选、模式与选择', text)
        self.assertNotIn('## 额外幻想补漏的本地执行', text)
        self.assertNotIn('## v7 逐集预览与正文冻结', text)

    def test_new_plan_cannot_reuse_old_packet(self):
        self.role_inputs('finisher')
        old = self.build()
        self.write('plan.json', '{"value":"changed plan","seconds":1.250000001}')
        with self.assertRaisesRegex(jp.PacketError, 'mismatch'):
            self.verify()
        new = self.build('packets/two.json')
        self.assertNotEqual(old['inputFingerprint'], new['inputFingerprint'])
        self.verify('packets/two.json')

    def test_required_material_review_rules_and_version_drift_are_rejected(self):
        packet = self.build()
        paths = [self.job / 'source.txt', self.job / 'assets.txt', self.job / 'review.md',
                 self.skill / jp.BASE_RULES[1],
                 self.skill / 'references/creative/skills/lighting-space-methods/SKILL.md',
                 self.skill / 'VERSION', self.skill / 'scripts/job_packet.py']
        for path in paths:
            with self.subTest(path=path.name):
                raw = path.read_bytes()
                path.write_bytes(raw + b'\nchanged')
                with self.assertRaises(jp.PacketError):
                    self.verify()
                path.write_bytes(raw)
                self.assertEqual(self.verify()['inputFingerprint'], packet['inputFingerprint'])

    def test_unreferenced_file_and_optional_method_change_do_not_invalidate(self):
        packet = self.build()
        self.write('unused-assets.json', '{"unrelated":"changed"}')
        path = self.skill / 'references/creative/skills/fantasy-expression-methods/SKILL.md'
        path.write_bytes(path.read_bytes() + b'\nchanged')
        self.assertEqual(self.verify()['inputFingerprint'], packet['inputFingerprint'])

    def test_changed_dependency_manifest_or_frozen_packet_is_rejected(self):
        self.build()
        target = self.job / 'packets/one.json'
        raw = target.read_bytes()
        data = jp.parse_json(raw, 'packet')
        data['task']['role'] = 'tampered'
        target.write_bytes(jp.canonical(data))
        with self.assertRaises(jp.PacketError):
            self.verify()
        target.write_bytes(raw)
        self.request['methods'] = ['fantasy-expression-methods']
        self.write('request.json', json.dumps(self.request))
        with self.assertRaises(jp.PacketError):
            self.verify()

    def test_changed_old_integer_clause_fails_closed_for_adapter_review(self):
        path = self.skill / jp.COMMON_RULE
        raw = path.read_text(encoding='utf-8')
        path.write_text(raw.replace('每个单元的镜长合计落在整数秒', '整数秒是硬性要求'), encoding='utf-8')
        with self.assertRaisesRegex(jp.PacketError, 'adapter needs review'):
            self.build()

    def test_budget_failure_never_writes_or_truncates(self):
        with self.assertRaisesRegex(jp.PacketError, 'no input truncated'):
            self.build(limit=100)
        self.assertFalse((self.job / 'packets/one.json').exists())
        self.build()
        original = (self.job / 'packets/one.json').read_bytes()
        with self.assertRaisesRegex(jp.PacketError, 'refusing overwrite'):
            self.build()
        self.assertEqual((self.job / 'packets/one.json').read_bytes(), original)

    def test_job_and_rule_path_escape_is_rejected(self):
        for path in ['../outside.txt', str(self.job / 'source.txt'), '']:
            self.request['inputs'][0]['path'] = path
            with self.assertRaises(jp.PacketError):
                self.build()
        self.request['inputs'][0]['path'] = 'source.txt'
        outside = self.root / 'outside'
        outside.mkdir()
        (outside / 'source.txt').write_text('outside')
        try:
            (self.job / 'escape').symlink_to(outside, target_is_directory=True)
        except OSError:
            self.skipTest('directory symlinks unavailable')
        self.request['inputs'][0]['path'] = 'escape/source.txt'
        with self.assertRaises(jp.PacketError):
            self.build()
        self.request['inputs'][0]['path'] = 'source.txt'
        with self.assertRaises(jp.PacketError):
            self.build('escape/out.json')
        version = self.skill / 'VERSION'
        version.unlink()
        version.symlink_to(outside / 'source.txt')
        with self.assertRaises(jp.PacketError):
            self.build()

    def test_strict_json_and_pointer_errors_fail_without_echoing_data(self):
        item = self.add('plan', 'plan.json', '{}', '/selected')
        for raw in ['{"secret":1,"secret":2}', '{"secret":NaN}', '{"secret":3}']:
            self.write('plan.json', raw)
            with self.assertRaises(jp.PacketError) as result:
                self.build()
            self.assertNotIn('secret', str(result.exception))
        self.write('plan.json', '{"selected":[1]}')
        item['pointer'] = '/selected/01'
        with self.assertRaises(jp.PacketError):
            self.build()

    def test_invalid_kinds_methods_roles_and_duplicate_keys_rejected(self):
        original = copy.deepcopy(self.request)
        for key, value in [('role', []), ('episode', True), ('methods', ['unknown']),
                           ('contracts', ['../outside']), ('version', 'series-context-request-v1')]:
            self.request = dict(original, **{key: value})
            with self.assertRaises(jp.PacketError):
                self.build()
        self.request = original
        self.request['inputs'][0]['kind'] = []
        with self.assertRaises(jp.PacketError):
            self.build()
        self.write('request.json', '{"version":1,"version":2}')
        with self.assertRaises(jp.PacketError):
            jp.assemble(self.job, 'request.json', skill_root=self.skill)

    def test_locked_scope_blocks_out_of_range_and_other_owner_change_reuses_packet(self):
        source = self.root / 'whole-source.txt'
        source.write_text('test_fixture complete source', encoding='utf-8')
        extraction = self.root / 'extraction'
        workspace.extract(source, extraction)
        for path in extraction.iterdir():
            shutil.copyfile(path, self.job / path.name)
        index = {'version': 'series-source-v1', 'sourceHash': workspace.sha(source),
                 'episodes': [{'number': n, 'scenes': [{'id': str(n) + '-1'}]} for n in [1, 2, 3]]}
        index_path = self.root / 'index.json'
        index_path.write_text(json.dumps(index), encoding='utf-8')
        workspace.init(self.job, index_path, 1, [1, 2])
        packet = self.build()
        state = json.loads((self.job / 'state.json').read_text(encoding='utf-8'))
        state['episodes']['2']['owner'] = 'another-worker'
        (self.job / 'state.json').write_text(json.dumps(state), encoding='utf-8')
        self.assertEqual(self.verify()['inputFingerprint'], packet['inputFingerprint'])
        self.request['episode'] = 3
        with self.assertRaisesRegex(jp.PacketError, 'outside locked'):
            self.build('packets/future.json')
        self.assertFalse((self.job / 'packets/future.json').exists())

    def test_cli_and_public_load_verified_interface(self):
        self.write('request.json', json.dumps(self.request, ensure_ascii=False))
        command = [sys.executable, str(BASE / 'scripts/job_packet.py'), 'build', '--workdir', str(self.job),
                   '--manifest', 'request.json', '--output', 'packets/cli.json']
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        info = json.loads(result.stdout)
        packet = jp.load_verified(self.job, 'packets/cli.json')
        self.assertEqual(info['inputFingerprint'], packet['inputFingerprint'])
        self.assertNotIn('不要改字', result.stdout)

    def test_historical_packet_survives_skill_upgrade_but_live_dispatch_does_not(self):
        packet = self.build()
        for relative in ['VERSION', 'scripts/job_packet.py', jp.COMMON_RULE]:
            path = self.skill / relative
            path.write_bytes(path.read_bytes() + b'\nupgraded')
        with self.assertRaises(jp.PacketError):
            self.verify()
        self.assertEqual(jp.load_frozen(self.job, 'packets/one.json'), packet)
        self.skill.rename(self.root / 'old-skill-removed')
        self.assertEqual(jp.load_frozen(self.job, 'packets/one.json'), packet)

    def test_historical_and_live_verification_both_reject_real_input_or_review_drift(self):
        self.role_inputs('finisher')
        self.build()
        for relative in ['source.txt', 'plan.json', 'selection.json', 'review.md', 'request.json']:
            with self.subTest(relative=relative):
                path = self.job / relative
                raw = path.read_bytes()
                path.write_bytes(raw + b'\n ')
                with self.assertRaises(jp.PacketError):
                    self.verify()
                with self.assertRaises(jp.PacketError):
                    jp.load_frozen(self.job, 'packets/one.json')
                path.write_bytes(raw)
        jp.load_frozen(self.job, 'packets/one.json')

    def test_historical_packet_rejects_tampering_even_after_resealing_outer_checksum(self):
        packet = self.build()
        target = self.job / 'packets/one.json'
        raw = target.read_bytes()
        modifications = [
            lambda p: p['task'].update(role='tampered'),
            lambda p: p['blobs'][p['rules'][0]['blob']].update(value='tampered rule'),
            lambda p: p['bindings'][0].update(originBytes=True),
            lambda p: p['rules'][0].update(blob='0' * 64),
            lambda p: p['rules'][0].update(path='../outside.md'),
            lambda p: p['rules'][0].update(sha256='not a hash'),
            lambda p: p['rules'][0].update(unknown=True),
            lambda p: p['skill'].update(version='different frozen version'),
            lambda p: p.update(inputFingerprint='0' * 64),
            lambda p: p.update(maxBytes=True),
            lambda p: p.update(scope={'scope': [1]}),
            lambda p: p['blobs'].update({jp.sha256(jp.canonical({'format': 'text', 'value': 'extra'})):
                                      {'format': 'text', 'value': 'extra'}}),
        ]
        for index, modify in enumerate(modifications):
            with self.subTest(index=index):
                changed = copy.deepcopy(packet)
                modify(changed)
                # Test zero deliberately keeps the old outer checksum. Others
                # exercise nested schema/content/fingerprint validation.
                if index:
                    changed['packetSha256'] = jp.sha256(jp.canonical({k: v for k, v in changed.items()
                                                                     if k != 'packetSha256'}))
                target.write_bytes(jp.canonical(changed))
                with self.assertRaises(jp.PacketError):
                    jp.load_frozen(self.job, 'packets/one.json')
        target.write_bytes(raw)
        jp.load_frozen(self.job, 'packets/one.json')

    def test_historical_packet_rechecks_current_workspace_scope(self):
        source = self.root / 'whole-source.txt'
        source.write_text('test_fixture complete source', encoding='utf-8')
        extraction = self.root / 'extraction'
        workspace.extract(source, extraction)
        for path in extraction.iterdir():
            shutil.copyfile(path, self.job / path.name)
        index = {'version': 'series-source-v1', 'sourceHash': workspace.sha(source),
                 'episodes': [{'number': n, 'scenes': [{'id': str(n) + '-1'}]} for n in [1, 2]]}
        index_path = self.root / 'index.json'
        index_path.write_text(json.dumps(index), encoding='utf-8')
        workspace.init(self.job, index_path, 1, None)
        self.build()
        jp.load_frozen(self.job, 'packets/one.json')
        state_path = self.job / 'state.json'
        state = json.loads(state_path.read_text(encoding='utf-8'))
        state['episodes']['2']['owner'] = 'other-worker'
        state_path.write_text(json.dumps(state), encoding='utf-8')
        jp.load_frozen(self.job, 'packets/one.json')
        state_path.unlink()
        with self.assertRaisesRegex(jp.PacketError, 'scope changed'):
            jp.load_frozen(self.job, 'packets/one.json')


if __name__ == '__main__':
    unittest.main()
