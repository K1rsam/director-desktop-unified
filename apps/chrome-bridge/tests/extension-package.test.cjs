const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const vm = require('node:vm');
const { fileURLToPath, pathToFileURL } = require('node:url');

function fixture(t, options = {}) {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'director-extension-package-'));
  t.after(() => fs.rmSync(temporary, { recursive: true, force: true }));
  const root = path.join(temporary, 'component & canonical');
  const extension = path.join(root, 'extension');
  const legacy = path.join(root, 'tools/chrome-bridge-extension');
  const manifest = { manifest_version: 3, version: '2.4.5', background: { service_worker: 'background.js' },
    content_scripts: [{ js: ['content.js'] }] };
  for (const [directory, marker] of [[extension, 'current-patched'], [legacy, 'old-unpatched']]) {
    fs.mkdirSync(directory, { recursive: true });
    fs.writeFileSync(path.join(directory, 'manifest.json'), JSON.stringify(manifest));
    fs.writeFileSync(path.join(directory, 'background.js'), marker);
    fs.writeFileSync(path.join(directory, 'content.js'), marker);
  }
  const registered = new Map();
  const archives = [];
  const router = { use() {}, get(route, ...handlers) { registered.set(route, handlers.at(-1)); }, post() {}, delete() {} };
  class HttpError extends Error { constructor(statusCode, message) { super(message); this.statusCode = statusCode; } }
  const context = vm.createContext({ URL, fileURLToPath, fs: fs.promises, path, HttpError, process: { env: {} },
    express: { Router: () => router },
    config: { publicBaseUrl: 'http://127.0.0.1:53179', dataDir: path.join(temporary, 'data'), bridgeToken: 'synthetic-test-token' },
    EXTENSION_COMPATIBILITY: { recommendedExtensionVersion: '2.4.5' }, BRIDGE_VERSION: '6.4.0',
    registerFullPowerBridgeRoutes() {}, registerPassivePromptRoutes() {}, registerWorkflowRoutes() {},
    writeZip: async (output, entries) => {
      archives.push({ output, entries: await Promise.all(entries.map(async entry => ({ name: entry.name,
        bytes: await fs.promises.readFile(entry.path, 'utf8'), source: entry.path }))) });
    },
  });
  const source = fs.readFileSync(path.join(__dirname, '../overrides/src/routes.js'), 'utf8')
    .replace(/^import[\s\S]*?from [^;]+;\r?\n/gm, '')
    .replace(/export (?=function|class)/g, '')
    .replace(/import\.meta\.url/g, JSON.stringify(pathToFileURL(path.join(root, 'src/routes.js')).href));
  vm.runInContext(source + ';this.createRouter=createRouter;', context);
  context.createRouter({ isLocalRequest: () => options.local !== false, health: () => options.health || { ok: false, clients: [] } }, {});
  async function request(route) {
    const output = { headers: {} };
    const response = { setHeader(key, value) { output.headers[key] = value; }, send(body) { output.body = body; },
      json(body) { output.body = body; }, download(file, name) { output.download = { file, name }; } };
    await registered.get(route)({}, response, error => { output.error = error; });
    return output;
  }
  return { request, root, extension, legacy, archives, manifest };
}

test('setup and export share the module-bound desktop extension, never the old tools copy or cwd', async t => {
  const f = fixture(t);
  const setup = await f.request('/setup');
  assert.equal(setup.error, undefined);
  assert.match(setup.body, /component &amp; canonical/);
  assert.ok(setup.body.includes(f.extension.replace(/&/g, '&amp;')));
  assert.ok(!setup.body.includes('tools/chrome-bridge-extension'));
  const download = await f.request('/extensions/chrome-bridge-extension.zip');
  assert.equal(download.error, undefined);
  assert.equal(download.download.name, 'chrome-bridge-extension.zip');
  const background = f.archives[0].entries.find(entry => entry.name === 'background.js');
  assert.equal(background.bytes, 'current-patched');
  assert.equal(background.source, path.join(f.extension, 'background.js'));
  assert.ok(f.archives[0].entries.every(entry => entry.source.startsWith(f.extension + path.sep)));
});

test('missing canonical package fails clearly even when a valid legacy tools package remains', async t => {
  const f = fixture(t);
  fs.renameSync(f.extension, path.join(f.root, 'removed-extension'));
  for (const route of ['/setup', '/extensions/chrome-bridge-extension.zip']) {
    const response = await f.request(route);
    assert.equal(response.error.statusCode, 503);
    assert.match(response.error.message, /Repair the desktop Chrome component/);
  }
  assert.equal(f.archives.length, 0);
});

test('incompatible or incomplete manifests never produce a misleading setup or export', async t => {
  for (const failure of ['version', 'missing-script', 'malformed', 'null', 'malformed-scripts']) {
    const f = fixture(t);
    if (failure === 'version') fs.writeFileSync(path.join(f.extension, 'manifest.json'), JSON.stringify({ ...f.manifest, version: '2.4.4' }));
    if (failure === 'missing-script') fs.unlinkSync(path.join(f.extension, 'content.js'));
    if (failure === 'malformed') fs.writeFileSync(path.join(f.extension, 'manifest.json'), '{');
    if (failure === 'null') fs.writeFileSync(path.join(f.extension, 'manifest.json'), 'null');
    if (failure === 'malformed-scripts') fs.writeFileSync(path.join(f.extension, 'manifest.json'), JSON.stringify({ ...f.manifest, content_scripts: [{ js: {} }] }));
    for (const route of ['/setup', '/extensions/chrome-bridge-extension.zip']) {
      assert.equal((await f.request(route)).error.statusCode, 503, failure);
    }
    assert.equal(f.archives.length, 0);
  }
});

test('linked subdirectories cannot introduce files outside the canonical package', async t => {
  const f = fixture(t);
  fs.symlinkSync(f.legacy, path.join(f.extension, 'foreign'), process.platform === 'win32' ? 'junction' : 'dir');
  const response = await f.request('/extensions/chrome-bridge-extension.zip');
  assert.equal(response.error.statusCode, 503);
  assert.match(response.error.message, /Linked files/);
  assert.equal(f.archives.length, 0);
});

test('setup and export retain their localhost access boundary', async t => {
  const f = fixture(t, { local: false });
  for (const route of ['/setup', '/extensions/chrome-bridge-extension.zip', '/setup/status']) {
    assert.equal((await f.request(route)).error.statusCode, 403);
  }
  assert.equal(f.archives.length, 0);
});

test('setup status handles disconnected arrays and absent or compact client counts without throwing', async t => {
  for (const clients of [[], 0, undefined]) {
    const f = fixture(t, { health: { ok: false, clients } });
    const response = await f.request('/setup/status');
    assert.equal(response.error, undefined);
    assert.equal(response.body.clients.length, 0);
    assert.match(response.body.error, /No compatible browser companion/);
  }
});

test('setup status preserves the precise incompatible-client reason', async t => {
  const f = fixture(t, { health: { ok: false, clients: [{ compatible: false,
    compatibility: { compatible: false, message: 'Synthetic outdated runtime' } }] } });
  const response = await f.request('/setup/status');
  assert.equal(response.error, undefined);
  assert.equal(response.body.clients.length, 1);
  assert.equal(response.body.error, 'Synthetic outdated runtime');
});
