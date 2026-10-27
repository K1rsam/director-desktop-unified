import hashlib
import json
from io import BytesIO
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'workspace.py'


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class WorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.job = self.root / 'job'
        source = self.root / 'source.txt'
        source.write_text('时间：夜晚\n全篇没有被截断。', encoding='utf-8')
        self.run_cli('extract', '--source', source, '--output-dir', self.job)
        self.source_hash = digest(source)

    def run_cli(self, *args, ok=True):
        p = subprocess.run([sys.executable, '-X', 'utf8', str(SCRIPT), *map(str, args)], capture_output=True, text=True, encoding='utf-8')
        self.assertEqual(p.returncode == 0, ok, p.stdout + p.stderr)
        return json.loads(p.stdout) if p.returncode == 0 else p.stderr

    def init(self, numbers=range(1, 9), workers=4, select=None):
        self.index = {'version': 'series-source-v1', 'sourceHash': self.source_hash,
                      'episodes': [{'number': n, 'scenes': [{'id': '%s-1' % n}]} for n in numbers]}
        self.index_path = self.root / 'index.json'
        self.index_path.write_text(json.dumps(self.index), encoding='utf-8')
        args = ['init', '--workdir', self.job, '--index', self.index_path, '--workers', workers]
        if select:
            args += ['--episodes', ','.join(map(str, select))]
        result = self.run_cli(*args, '--full')
        # Original suite exercises existing v1 workspaces, which must remain readable
        # without silently rewriting their author identities or historical evidence.
        state_path = self.job / 'state.json'
        state = json.loads(state_path.read_text())
        state.pop('dispatch', None)
        state_path.write_text(json.dumps(state))
        return result

    def claim(self, group, ok=True):
        return self.run_cli('claim', '--workdir', self.job, '--group', group, '--owner', 'agent', ok=ok)

    def evidence(self, episode):
        state = self.run_cli('status', '--workdir', self.job)
        path = self.job / ('ep%s.json' % episode)
        artifacts = {}
        for stage in ['intake', 'assets', 'direction', 'delivery']:
            f = self.job / ('ep%s-%s.txt' % (episode, stage))
            f.write_text('test_fixture: %s %s' % (episode, stage), encoding='utf-8')
            artifacts[stage] = f.name
        payload = {'version': 'series-episode-evidence-v1', 'episode': episode,
                   'sourceHash': self.source_hash, 'sourceIndexHash': state['sourceIndexHash'],
                   'scenes': ['%s-1' % episode], 'artifacts': artifacts,
                   'adopted': artifacts['direction'], 'reviewMode': 'independent'}
        ep_state = state['episodes'][str(episode)]
        if ep_state.get('claimedUpstreamHash'):
            payload['entryReview'] = {'upstreamHash': ep_state['claimedUpstreamHash'],
                                      'notes': 'test_fixture: read adopted predecessor and checked entry state'}
        path.write_text(json.dumps(payload), encoding='utf-8')
        return path

    def complete(self, ep):
        return self.run_cli('complete', '--workdir', self.job, '--episode', ep,
                            '--owner', 'agent', '--evidence', self.evidence(ep))

    def test_four_contiguous_groups_and_serial_progress(self):
        state = self.init()
        self.assertEqual([g['episodes'] for g in state['groups']], [[1, 2], [3, 4], [5, 6], [7, 8]])
        self.assertEqual(self.claim(1)['episode'], 1)
        self.claim(1, ok=False)
        self.complete(1)
        self.assertEqual(self.claim(1)['episode'], 2)
        self.complete(2)
        self.assertIsNone(self.claim(1)['episode'])
        self.run_cli('init', '--workdir', self.job, '--index', self.index_path, ok=False)

    def test_eight_episode_completion_requires_cross_group_reconciliation(self):
        self.init()
        for group in range(1, 5):
            for ep in (group * 2 - 1, group * 2):
                self.assertEqual(self.claim(group)['episode'], ep)
                self.complete(ep)
        state = self.run_cli('status', '--workdir', self.job)
        self.assertTrue(state['textComplete'])
        self.assertFalse(state['readyForDelivery'])
        for current in (3, 5, 7):
            report = self.job / ('boundary-%s.json' % current)
            report.write_text(json.dumps({
                'version': 'series-boundary-v1', 'upstream': current - 1, 'current': current,
                'upstreamHash': state['episodes'][str(current - 1)]['completion']['adopted']['sha256'],
                'currentHash': state['episodes'][str(current)]['completion']['adopted']['sha256'],
                'outcome': 'pass', 'notes': 'test_fixture: actual input/output reviewed'
            }), encoding='utf-8')
            self.run_cli('reconcile', '--workdir', self.job, '--episode', current, '--evidence', report)
        self.assertTrue(self.run_cli('status', '--workdir', self.job)['readyForDelivery'])
        self.assertIsNone(self.claim(4)['episode'])

    def test_worker_counts_keep_noncontiguous_source_order(self):
        self.init([3, 9, 2, 12, 5], workers=3)
        state = self.run_cli('status', '--workdir', self.job)
        self.assertEqual([g['episodes'] for g in state['groups']], [[3, 9], [2, 12], [5]])

    def test_two_workers_and_less_than_four_episodes(self):
        self.init([1, 2, 3], workers=2)
        self.assertEqual([g['episodes'] for g in self.run_cli('status', '--workdir', self.job)['groups']], [[1, 2], [3]])

    def test_claim_is_not_reassigned_after_restart(self):
        self.init()
        self.claim(1)
        self.assertEqual(self.run_cli('status', '--workdir', self.job)['episodes']['1']['status'], 'in_flight')
        self.claim(1, ok=False)
        report = self.job / 'recovery.txt'
        report.write_text('agent terminated; retained local draft inspected', encoding='utf-8')
        self.run_cli('resume', '--workdir', self.job, '--episode', 1, '--evidence', report)
        self.assertEqual(self.claim(1)['episode'], 1)

    def test_block_is_local(self):
        self.init()
        self.claim(1)
        report = self.job / 'block.txt'
        report.write_text('missing source fact', encoding='utf-8')
        self.run_cli('block', '--workdir', self.job, '--episode', 1, '--owner', 'agent', '--evidence', report)
        self.claim(1, ok=False)
        self.assertEqual(self.claim(2)['episode'], 3)

    def test_late_boundary_reconciliation_and_tamper(self):
        self.init([1, 2], workers=4)
        self.claim(2)
        self.complete(2)
        state = self.run_cli('status', '--workdir', self.job)
        self.assertEqual(state['episodes']['2']['boundary'], 'PRELIMINARY')
        self.claim(1)
        self.complete(1)
        state = self.run_cli('status', '--workdir', self.job)
        self.assertTrue(state['textComplete'])
        self.assertFalse(state['readyForDelivery'])
        upstream = state['episodes']['1']['completion']['adopted']['sha256']
        current = state['episodes']['2']['completion']['adopted']['sha256']
        report = self.job / 'boundary.json'
        report.write_text(json.dumps({'version': 'series-boundary-v1', 'upstream': 1, 'current': 2,
                                     'upstreamHash': upstream, 'currentHash': current, 'outcome': 'pass',
                                     'notes': 'test_fixture continuity checked'}), encoding='utf-8')
        self.run_cli('reconcile', '--workdir', self.job, '--episode', 2, '--evidence', report)
        self.assertTrue(self.run_cli('status', '--workdir', self.job)['readyForDelivery'])
        (self.job / 'ep1-direction.txt').write_text('changed', encoding='utf-8')
        state = self.run_cli('status', '--workdir', self.job)
        self.assertFalse(state['readyForDelivery'])
        self.assertFalse(state['textComplete'])
        self.claim(1, ok=False)

    def test_changed_source_or_index_rejected(self):
        self.init()
        (self.job / 'source-index.json').write_text('{}', encoding='utf-8')
        self.run_cli('status', '--workdir', self.job, ok=False)

    def test_selected_episode_has_out_of_scope_predecessor(self):
        self.init([1, 2, 3], select=[2])
        self.claim(1)
        self.complete(2)
        state = self.run_cli('status', '--workdir', self.job)
        self.assertEqual(state['episodes']['2']['boundary'], 'NOT_IN_SCOPE')
        self.assertTrue(state['readyForDelivery'])

    def test_scope_gap_does_not_invent_predecessor(self):
        self.init([1, 2, 3], select=[1, 3])
        self.assertEqual(self.claim(2)['boundary'], 'NOT_IN_SCOPE')

    def test_original_source_change_rejected(self):
        self.init()
        (self.job / 'source-original.txt').write_text('changed', encoding='utf-8')
        self.run_cli('status', '--workdir', self.job, ok=False)

    def test_escape_evidence_rejected(self):
        self.init()
        self.claim(1)
        outside = self.root / 'outside.txt'
        outside.write_text('escape', encoding='utf-8')
        self.run_cli('block', '--workdir', self.job, '--episode', 1, '--owner', 'agent', '--evidence', outside, ok=False)

    def test_less_episodes_than_workers(self):
        state = self.init([8, 11, 20], workers=4)
        self.assertEqual([g['episodes'] for g in state['groups']], [[8], [11], [20]])

    def test_complete_requires_full_evidence(self):
        self.init()
        self.claim(1)
        evidence = self.evidence(1)
        data = json.loads(evidence.read_text())
        data['scenes'] = []
        evidence.write_text(json.dumps(data))
        self.run_cli('complete', '--workdir', self.job, '--episode', 1,
                     '--owner', 'agent', '--evidence', evidence, ok=False)

    def test_concurrent_claim_once(self):
        self.init()
        command = [sys.executable, '-X', 'utf8', str(SCRIPT), 'claim', '--workdir', str(self.job), '--group', '1', '--owner', 'agent']
        ps = [subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(4)]
        for p in ps:
            p.communicate()
        self.assertEqual(sum(p.returncode == 0 for p in ps), 1)

    def test_serial_boundary_needs_reading_evidence(self):
        self.init([1, 2], workers=1)
        self.claim(1)
        self.complete(1)
        self.claim(1)
        evidence = self.evidence(2)
        data = json.loads(evidence.read_text())
        del data['entryReview']
        evidence.write_text(json.dumps(data), encoding='utf-8')
        self.run_cli('complete', '--workdir', self.job, '--episode', 2,
                     '--owner', 'agent', '--evidence', evidence, ok=False)
        data['entryReview'] = {'upstreamHash': 'wrong', 'notes': 'claimed checked'}
        evidence.write_text(json.dumps(data), encoding='utf-8')
        self.run_cli('complete', '--workdir', self.job, '--episode', 2,
                     '--owner', 'agent', '--evidence', evidence, ok=False)
        self.complete(2)
        self.assertTrue(self.run_cli('status', '--workdir', self.job)['readyForDelivery'])

    def test_docx_revision_textbox_table_and_breaks(self):
        source = self.root / 'revision.docx'
        xml = '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>开头</w:t><w:tab/><w:t>制表</w:t><w:br/><w:t>换行</w:t></w:r><w:del><w:r><w:delText>已删标准</w:delText><w:t>已删嵌套</w:t></w:r><w:ins><w:r><w:t>删除内插入</w:t></w:r></w:ins></w:del><w:ins><w:r><w:t>保留插入</w:t></w:r></w:ins><w:r><w:drawing><w:txbxContent><w:p><w:r><w:t>文本框</w:t></w:r></w:p></w:txbxContent><w:t>绘图文字</w:t></w:drawing></w:r></w:p><w:del><w:p><w:r><w:t>整段删除</w:t></w:r></w:p></w:del><w:tbl><w:tr><w:tc><w:p><w:r><w:t>表格</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:moveFrom><w:r><w:t>原移动位置</w:t></w:r></w:moveFrom><w:moveTo><w:r><w:t>新移动位置</w:t></w:r></w:moveTo><w:r><w:t>结尾</w:t></w:r></w:p></w:body></w:document>'
        with zipfile.ZipFile(source, 'w') as z:
            z.writestr('word/document.xml', xml)
        job = self.root / 'revision-job'
        result = self.run_cli('extract', '--source', source, '--output-dir', job)
        self.assertEqual((job / 'source-body.txt').read_text(), '开头\t制表\n换行保留插入\n表格\n新移动位置结尾\n')
        self.assertIn('deleted revision text', result['notExtracted'])

    def test_init_same_index_path_and_concurrent_once(self):
        index = self.job / 'source-index.json'
        index.write_text(json.dumps({'version': 'series-source-v1', 'sourceHash': self.source_hash,
                                     'episodes': [{'number': 1, 'scenes': [{'id': '1-1'}]}]}), encoding='utf-8')
        initial = index.read_bytes()
        command = [sys.executable, '-X', 'utf8', str(SCRIPT), 'init', '--workdir', str(self.job), '--index', str(index)]
        ps = [subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(3)]
        for p in ps:
            p.communicate()
        self.assertEqual(sum(p.returncode == 0 for p in ps), 1)
        self.assertEqual(index.read_bytes(), initial)
        self.assertEqual(self.run_cli('status', '--workdir', self.job, '--full')['scope'], [1])

    def test_docx_preserves_paragraph_table_order(self):
        source = self.root / 'source.docx'
        xml = '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>开头</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>表内</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>结尾</w:t></w:r></w:p></w:body></w:document>'
        with zipfile.ZipFile(source, 'w') as z:
            z.writestr('word/document.xml', xml)
        job = self.root / 'docx-job'
        result = self.run_cli('extract', '--source', source, '--output-dir', job)
        self.assertEqual((job / 'source-body.txt').read_text(), '开头\n表内\n结尾\n')
        self.assertEqual(result['sha256'], digest(source))
        self.assertIn('comments', result['notExtracted'])
        self.run_cli('extract', '--source', source, '--output-dir', job, ok=False)

    def test_docx_preserves_special_hyphens_and_reports_unmapped_symbol(self):
        source = self.root / 'inline-symbols.docx'
        xml = '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>密码 A</w:t><w:noBreakHyphen/><w:t>B；soft</w:t><w:softHyphen/><w:t>hyphen；标志</w:t><w:sym w:font="Wingdings" w:char="F0FC"/><w:t>。</w:t></w:r></w:p></w:body></w:document>'
        with zipfile.ZipFile(source, 'w') as z:
            z.writestr('word/document.xml', xml)
        job = self.root / 'symbol-job'
        result = self.run_cli('extract', '--source', source, '--output-dir', job)
        self.assertEqual((job / 'source-body.txt').read_text(), '密码 A\u2011B；soft\u00adhyphen；标志。\n')
        missing = result.get('notExtractedDetails', [])
        self.assertTrue(any(item.get('kind') == 'symbol' and item.get('path') == '/w:body/w:p[1]/w:r[1]/w:sym[1]'
                            and item.get('font') == 'Wingdings' and item.get('char') == 'F0FC' for item in missing), missing)

    def test_docx_extraction_uses_the_same_snapshot_as_saved_original(self):
        source = self.root / 'changing.docx'
        def docx_bytes(text):
            stream = BytesIO()
            with zipfile.ZipFile(stream, 'w') as z:
                z.writestr('word/document.xml', '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>' + text + '</w:t></w:r></w:p></w:body></w:document>')
            return stream.getvalue()
        original, replacement = docx_bytes('原始采用源'), docx_bytes('后来的保存版本')
        source.write_bytes(original)
        read_bytes = Path.read_bytes
        def read_then_save(path):
            raw = read_bytes(path)
            if path == source:
                source.write_bytes(replacement)
            return raw
        extract = runpy.run_path(str(SCRIPT))['extract']
        job = self.root / 'changing-source-job'
        with patch.object(Path, 'read_bytes', read_then_save):
            result = extract(source, job)
        self.assertEqual((job / 'source-original.docx').read_bytes(), original)
        self.assertEqual(result['sha256'], hashlib.sha256(original).hexdigest())
        self.assertEqual((job / 'source-body.txt').read_text(), '原始采用源\n')

    def test_docx_reports_altchunk_main_body_with_relationship(self):
        source = self.root / 'altchunk.docx'
        xml = '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:r><w:t>前言</w:t></w:r></w:p><w:altChunk r:id="script"/></w:body></w:document>'
        with zipfile.ZipFile(source, 'w') as z:
            z.writestr('word/document.xml', xml)
            z.writestr('word/_rels/document.xml.rels', '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="script" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/aFChunk" Target="script.html"/></Relationships>')
            z.writestr('word/script.html', '<html><body>第1集：完整剧本正文</body></html>')
        job = self.root / 'altchunk-job'
        result = self.run_cli('extract', '--source', source, '--output-dir', job)
        missing = result.get('notExtractedDetails', [])
        self.assertTrue(any(item.get('kind') == 'altChunk' and item.get('path') == '/w:body/w:altChunk[1]'
                            and item.get('relationshipId') == 'script' and item.get('target') == 'script.html' for item in missing), missing)
        self.assertIn('altChunk', result['notExtracted'])

    def test_docx_reports_unsupported_body_containers_without_reading_both_alternatives(self):
        source = self.root / 'body-containers.docx'
        xml = '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:body><w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="3"/></w:numPr></w:pPr><w:r><w:t>正文</w:t></w:r><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></w:p><mc:AlternateContent><mc:Choice Requires="w"><w:p><w:r><w:t>选择分支</w:t></w:r></w:p></mc:Choice><mc:Fallback><w:p><w:r><w:t>后备分支</w:t></w:r></w:p></mc:Fallback></mc:AlternateContent></w:body></w:document>'
        with zipfile.ZipFile(source, 'w') as z:
            z.writestr('word/document.xml', xml)
        job = self.root / 'body-container-job'
        result = self.run_cli('extract', '--source', source, '--output-dir', job)
        self.assertEqual((job / 'source-body.txt').read_text(), '正文\n')
        missing = result.get('notExtractedDetails', [])
        self.assertEqual({item['kind'] for item in missing}, {'numbering', 'math', 'alternateContent'})
        self.assertTrue(all(item.get('path') and item.get('part') == 'word/document.xml' for item in missing))

    def test_docx_reports_numbering_inherited_from_used_and_default_paragraph_styles(self):
        source = self.root / 'style-numbering.docx'
        xml = '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Episode"/></w:pPr><w:r><w:t>门厅</w:t></w:r></w:p><w:p><w:r><w:t>默认样式段落</w:t></w:r></w:p></w:body></w:document>'
        styles = '<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="NumberedBase"><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Episode"><w:basedOn w:val="NumberedBase"/></w:style><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:basedOn w:val="NumberedBase"/></w:style></w:styles>'
        numbering = '<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="第%1集"/></w:lvl></w:abstractNum><w:num w:numId="7"><w:abstractNumId w:val="1"/></w:num></w:numbering>'
        with zipfile.ZipFile(source, 'w') as z:
            z.writestr('word/document.xml', xml)
            z.writestr('word/styles.xml', styles)
            z.writestr('word/numbering.xml', numbering)
        job = self.root / 'style-numbering-job'
        result = self.run_cli('extract', '--source', source, '--output-dir', job)
        missing = result.get('notExtractedDetails', [])
        self.assertIn('numbering', result['notExtracted'])
        records = [item for item in missing if item['kind'] == 'numbering']
        self.assertEqual({item.get('styleId') for item in records}, {'Episode', 'Normal'})
        self.assertEqual({item.get('path') for item in records}, {'/w:body/w:p[1]', '/w:body/w:p[2]'})
        self.assertTrue(all(item.get('inheritedFromStyle') == 'NumberedBase' and item.get('numId') == '7'
                            and item.get('stylePart') == 'word/styles.xml' for item in records), records)

    def test_resume_predecessor_cannot_overlap_running_episode_in_same_group(self):
        self.init([1, 2, 3], workers=1)
        self.claim(1)
        self.complete(1)
        self.claim(1)
        report = self.job / 'recovery.md'
        report.write_text('原作者已停止，保留稿已核对，准备局部返修。', encoding='utf-8')
        before = (self.job / 'state.json').read_bytes()
        self.run_cli('resume', '--workdir', self.job, '--episode', 1, '--evidence', report, ok=False)
        self.assertEqual((self.job / 'state.json').read_bytes(), before)
        self.claim(1, ok=False)
        self.run_cli('resume', '--workdir', self.job, '--episode', 2, '--evidence', report)
        self.run_cli('resume', '--workdir', self.job, '--episode', 1, '--evidence', report)
        self.assertEqual(self.claim(1)['episode'], 1)
        state = self.run_cli('status', '--workdir', self.job)
        self.assertEqual(sum(ep['status'] == 'in_flight' for ep in state['episodes'].values()), 1)

    def test_claim_cannot_reuse_stale_serial_boundary_until_reconciled(self):
        self.init([1, 2, 3], workers=1)
        self.claim(1)
        self.complete(1)
        self.claim(1)
        self.complete(2)
        report = self.job / 'recovery.md'
        report.write_text('第1集出口需局部返修，作者已结束。', encoding='utf-8')
        self.run_cli('resume', '--workdir', self.job, '--episode', 1, '--evidence', report)
        self.claim(1)
        evidence = self.evidence(1)
        (self.job / 'ep1-direction.txt').write_text('返修后出口：左手不再持灯。', encoding='utf-8')
        self.run_cli('complete', '--workdir', self.job, '--episode', 1, '--owner', 'agent', '--evidence', evidence)
        state = self.run_cli('status', '--workdir', self.job)
        self.assertEqual(state['episodes']['2']['boundary'], 'STALE')
        self.claim(1, ok=False)
        boundary_report = self.job / 'boundary.json'
        boundary_report.write_text(json.dumps({'version': 'series-boundary-v1', 'upstream': 1, 'current': 2,
                                              'upstreamHash': state['episodes']['1']['completion']['adopted']['sha256'],
                                              'currentHash': state['episodes']['2']['completion']['adopted']['sha256'],
                                              'outcome': 'pass', 'notes': '当前第2集稿已按第1集实际出口复核，无需修改。'}), encoding='utf-8')
        self.run_cli('reconcile', '--workdir', self.job, '--episode', 2, '--evidence', boundary_report)
        self.assertEqual(self.claim(1)['episode'], 3)

    def test_unbound_preliminary_group_can_continue_serially(self):
        self.init([1, 2, 3, 4], workers=2)
        self.assertEqual(self.claim(2)['episode'], 3)
        self.complete(3)
        self.assertEqual(self.claim(2)['episode'], 4)
        self.complete(4)
        state = self.run_cli('status', '--workdir', self.job)
        self.assertEqual(state['episodes']['3']['boundary'], 'PRELIMINARY')
        self.assertEqual(state['episodes']['4']['boundary'], 'ACTUAL_RECONCILED')

    def test_reopening_reconciled_cross_group_predecessor_stops_reuse(self):
        self.init([1, 2, 3, 4], workers=2)
        self.claim(1)
        self.complete(1)
        self.claim(1)
        self.complete(2)
        self.claim(2)
        state = self.complete(3)
        boundary_report = self.job / 'boundary.json'
        boundary_report.write_text(json.dumps({'version': 'series-boundary-v1', 'upstream': 2, 'current': 3,
                                              'upstreamHash': state['episodes']['2']['completion']['adopted']['sha256'],
                                              'currentHash': state['episodes']['3']['completion']['adopted']['sha256'],
                                              'outcome': 'pass', 'notes': '已核实际入口和出口。'}), encoding='utf-8')
        self.run_cli('reconcile', '--workdir', self.job, '--episode', 3, '--evidence', boundary_report)
        report = self.job / 'recovery.md'
        report.write_text('第2集需要返修，作者已结束。', encoding='utf-8')
        self.run_cli('resume', '--workdir', self.job, '--episode', 2, '--evidence', report)
        self.claim(2, ok=False)


if __name__ == '__main__':
    unittest.main()
