"""One-call preparation and real adoption; synthetic story, no quality claims."""
import copy
from pathlib import Path
import sys
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE / 'scripts'))
import job_packet as jp
import prepare_episode as pe
import test_episode_execution as fixtures


class PrepareEpisodeTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.EpisodeExecutionTests()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.setUp()
        self.job = self.fixture.job
        self.fixture.bundle['episodes'][0]['settings']['fantasyMode'] = 'off'
        self.fixture.save('bundle.json', self.fixture.bundle)

    def prepare(self, role='author', output='prepared', **kwargs):
        return pe.prepare(self.job, 'bundle.json', 'registry', role, output, **kwargs)

    def test_direct_author_uses_exact_source_designs_and_no_extra_reports(self):
        f = self.fixture
        for key in ('plan', 'selection', 'draft'):
            f.bundle['episodes'][0][key] = {}
        f.save('bundle.json', f.bundle)
        before = (self.job / 'bundle.json').read_bytes()
        result = self.prepare()
        packet = jp.load_verified(self.job, result['packet'])
        kinds = {b['kind'] for b in packet['bindings']}
        self.assertFalse({'review', 'state', 'entry', 'plan', 'draft'} & kinds)
        self.assertIsNone(packet['review'])
        source = next(b for b in packet['bindings'] if b['kind'] == 'source')
        self.assertEqual(packet['blobs'][source['blob']]['value'], f.bundle['episodes'][0]['source'])
        materials = f.read('prepared/materials.json')
        registry = f.registry()
        for row in materials['assets']['assets']:
            self.assertEqual(row, next(a for a in registry['assets'] if a['id'] == row['id']))
        self.assertNotIn('visualBaseline', materials['direction'])
        self.assertEqual(before, (self.job / 'bundle.json').read_bytes())
        self.assertEqual(jp.load_frozen(self.job, result['packet']), packet)
        self.assertTrue((self.job / result['read']).is_file())
        with self.assertRaisesRegex(jp.PacketError, 'output exists'):
            self.prepare()

    def test_one_existing_effective_context_excludes_history_without_rewriting(self):
        f = self.fixture
        context = {'effective': {'entry': '右手仍握钥匙', 'constraint': '面罩遮口鼻，不露嘴'},
                   'history': '旧核查过程不供本岗'}
        f.save('decisions.json', context)
        result = self.prepare(context='decisions.json', context_pointer='/effective')
        packet = jp.load_verified(self.job, result['packet'])
        binding = next(b for b in packet['bindings'] if b['kind'] == 'state')
        self.assertEqual(packet['blobs'][binding['blob']]['value'], context['effective'])
        view = (self.job / result['read']).read_text(encoding='utf-8')
        self.assertIn('面罩遮口鼻', view)
        self.assertNotIn('旧核查过程不供本岗', view)
        self.assertEqual(f.read('decisions.json'), context)

    def test_dependency_closure_retains_parent_and_skips_unrelated_history(self):
        f = self.fixture
        head = f.read('registry/current.json')
        snapshot_path = 'registry/v%06d.json' % head['revision']
        registry = f.read(snapshot_path)
        child = registry['assets'][0]
        parent = copy.deepcopy(child)
        parent.update(id='A9000', key='当前造型母版CH', name='当前造型母版')
        parent['design']['dependencies'] = []
        parent['design']['unknownConstraint'] = '必须保持可抓握的袖口'
        child['design']['dependencies'] = [{'assetId': parent['id'], 'assetRevision': parent['revision'], 'purpose': '直接造型继承'}]
        unrelated = copy.deepcopy(parent)
        unrelated.update(id='A9001', key='无关资产CH', name='无关资产')
        registry['assets'] += [parent, unrelated]
        registry['imageDesigns'] = {'unused': '不供阅读的制图文字'}
        registry['requests'].append({'history': '不供阅读的请求历史'})
        f.save(snapshot_path, registry)
        head['sha256'] = jp.sha256((self.job / snapshot_path).read_bytes())
        f.save('registry/current.json', head)
        result = self.prepare()
        selected = f.read('prepared/materials.json')['assets']['assets']
        self.assertIn(parent, selected)
        self.assertNotIn('A9001', {a['id'] for a in selected})
        view = (self.job / result['read']).read_text(encoding='utf-8')
        self.assertIn('必须保持可抓握的袖口', view)
        self.assertNotIn('不供阅读的请求历史', view)
        self.assertNotIn('不供阅读的制图文字', view)

    def test_wrong_asset_revision_is_rejected_before_dispatch(self):
        self.fixture.bundle['assets'][0]['assetRevision'] += 1
        self.fixture.save('bundle.json', self.fixture.bundle)
        with self.assertRaisesRegex(jp.PacketError, 'changed required asset'):
            self.prepare()
        self.assertFalse((self.job / 'prepared').exists())

    def test_finisher_preparation_can_complete_real_local_adoption(self):
        f = self.fixture
        result = self.prepare(role='finisher')
        request = f.read('execution-request.json')
        request['packet'] = result['packet']
        f.save('execution-request.json', request)
        f.run_execution()
        self.assertTrue((self.job / 'adoption/adoption.json').exists())
        f.verify_execution()
        reviewer = self.prepare(role='reviewer', output='reviewer')
        kinds = {b['kind'] for b in jp.load_verified(self.job, reviewer['packet'])['bindings']}
        self.assertTrue({'source', 'plan', 'selection', 'draft'} <= kinds)

    def test_cli_needs_no_individual_preflight_manifest_or_view_calls(self):
        result = self.fixture.cli('prepare_episode.py', '--workdir', self.job,
                                  '--base', 'bundle.json', '--registry', 'registry',
                                  '--output', 'cli-author', '--role', 'author')
        self.assertEqual(result['episode'], 1)
        self.assertTrue((self.job / result['read']).is_file())


if __name__ == '__main__':
    unittest.main()
