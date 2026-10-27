"""Season boundaries constrain actual dispatch and frozen worker inputs."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

BASE = Path(__file__).resolve().parents[1]

class SeasonWorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.job = self.root / 'job'
        source = self.root / 'source.txt'
        source.write_text('test_fixture: three complete original episodes\n', encoding='utf-8')
        self.cli('extract', '--source', source, '--output-dir', self.job)
        self.source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
        self.reading = [{'number': n, 'sceneIds': [f'{n}-1']} for n in (1, 2, 3)]
        self.scope = {'version':'series-season-scope-v1', 'id':'s1', 'revision':1,
                      'seasonNumber':1, 'status':'locked', 'sourceHash':self.source_hash,
                      'readingScope':self.reading, 'episodes':self.reading[:2],
                      'target':{'minSeconds':7200,'maxSeconds':9000,'planningSeconds':8100},
                      'ending':{'episode':2,'sceneId':'2-1','sourceRef':'source-body.txt:L1',
                                'closure':'源文冲突兑现','hook':'后续原生悬念',
                                'estimate':{'minSeconds':7600,'maxSeconds':8500}},
                      'confirmation':{'kind':'delegated','basis':'test_fixture 授权'}}
        self.index = {'version':'series-source-v1','sourceHash':self.source_hash,
                      'episodes':[{'number':n,'scenes':[{'id':f'{n}-1'}]} for n in (1,2,3)],
                      'productionScope':self.scope}
        self.index_path = self.root / 'index.json'

    def cli(self, *args, ok=True, script='workspace.py'):
        result = subprocess.run([sys.executable, str(BASE/'scripts'/script), *map(str,args)], capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        return json.loads(result.stdout) if ok else result.stderr

    def init(self, **kwargs):
        self.index_path.write_text(json.dumps(self.index), encoding='utf-8')
        return self.cli('init','--workdir',self.job,'--index',self.index_path,'--workers',1,'--full',**kwargs)

    def test_embedded_scope_projects_real_dispatch_not_only_metadata(self):
        state = self.init()
        self.assertEqual(state['scope'], [1,2])
        self.assertEqual(state['groups'][0]['episodes'], [1,2])
        projected = json.loads((self.job/'source-index.json').read_text())
        self.assertEqual([e['number'] for e in projected['episodes']], [1,2])
        self.assertEqual(projected['readingScope'], self.reading)
        self.assertEqual(state['productionScope'], self.scope)

    def test_unconfirmed_season_does_not_create_a_production_workspace(self):
        self.scope['status']='proposed'
        self.init(ok=False)
        self.assertFalse((self.job/'state.json').exists())

    def test_sequel_must_consume_real_closed_handoff_before_dispatch(self):
        self.scope.update(id='s2',seasonNumber=2,
                          continuation={'previousScopeHash':'f'*64,'previousSeasonId':'s1','nextEpisode':1})
        error = self.init(ok=False)
        self.assertRegex(error, 'previous-handoff|上季|闭季')
        self.assertFalse((self.job/'state.json').exists())

    def test_explicit_dispatch_subset_cannot_reintroduce_future_episode(self):
        self.index_path.write_text(json.dumps(self.index), encoding='utf-8')
        self.cli('init','--workdir',self.job,'--index',self.index_path,'--workers',1,'--episodes','3',ok=False)

    def test_worker_packet_blocks_future_episode_even_with_valid_hashed_files(self):
        self.init()
        request={'version':'series-context-request-v1','episode':3,'role':'author','inputs':[]}
        for kind in ('source','direction','state','assets','entry'):
            raw=('test_fixture '+kind).encode()
            path=kind+'.txt';(self.job/path).write_bytes(raw)
            request['inputs'].append({'kind':kind,'path':path,'sha256':hashlib.sha256(raw).hexdigest()})
        (self.job/'request.json').write_text(json.dumps(request),encoding='utf-8')
        self.cli('build','--workdir',self.job,'--manifest','request.json','--output','out.json',ok=False,script='context_packet.py')
        self.assertFalse((self.job/'out.json').exists())

    def test_tampered_state_cannot_expand_frozen_source_scope(self):
        self.init();path=self.job/'state.json';state=json.loads(path.read_text())
        state['scope'].append(3);state['episodes']['3']={'status':'pending','group':1}
        state['groups'][0]['episodes'].append(3);path.write_text(json.dumps(state),encoding='utf-8')
        self.cli('claim','--workdir',self.job,'--group',1,'--owner','worker',ok=False)

if __name__ == '__main__': unittest.main()
