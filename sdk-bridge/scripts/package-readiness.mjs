// No network, install scripts, credential reads or runtime execution. Packaging
// fails loudly unless npm ci --ignore-scripts --omit=optional was done first.
import { createHash } from 'node:crypto';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const sdkRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const pinned = Object.freeze({
  version: '0.160.1',
  implementation: 'd62ed107033bdba802b283c77d875e4bec3deb2704a910bb7e3f95059473b16f',
  package: '5ffdee6d6135684e502d2d6010bae3690c0e2acb44e95d1e27c14fc5bbfccf30',
  license: 'd17f227e4df5da1600391338865ce0f3055211760a36688f816941d58232d8dc',
});
export const resourceFiles = Object.freeze([
  'src/health.mjs', 'src/bridge.mjs', 'THIRD_PARTY_NOTICES.md', 'package-lock.json',
  'node_modules/@openai/codex-sdk/package.json',
  'node_modules/@openai/codex-sdk/dist/index.js',
  'node_modules/@openai/codex-sdk/LICENSE',
  'src/main.mjs', 'src/native-runtime.mjs', 'src/visual-review.mjs',
]);
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
async function safeDirectory(directory, root) {
  if (directory === root) return;
  const parent = path.dirname(directory);
  if (parent === directory || !directory.startsWith(root + path.sep)) throw new Error('Invalid staging path');
  await safeDirectory(parent, root);
  const existing = await fs.lstat(directory).catch(error => { if (error.code !== 'ENOENT') throw error; });
  if (existing && (!existing.isDirectory() || existing.isSymbolicLink())) throw new Error('Packaging directory must not contain symlinks');
  if (!existing) await fs.mkdir(directory);
}
async function verifyExistingFiles(directory, prefix = '') {
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const relative = prefix + entry.name;
    if (entry.isSymbolicLink()) throw new Error(`Unsafe staged symlink: ${relative}`);
    if (entry.isDirectory()) await verifyExistingFiles(path.join(directory, entry.name), relative + '/');
    else if (!resourceFiles.includes(relative) && !['.gitignore', 'manifest.json'].includes(relative)) {
      throw new Error(`Unexpected resource in readiness bundle: ${relative}`);
    }
  }
}
export async function packageReadiness(destination) {
  const packageBytes = await fs.readFile(path.join(sdkRoot, resourceFiles[4]));
  const implementation = await fs.readFile(path.join(sdkRoot, resourceFiles[5]));
  const metadata = JSON.parse(packageBytes);
  if (metadata.name !== '@openai/codex-sdk' || metadata.version !== pinned.version
    || hash(packageBytes) !== pinned.package || hash(implementation) !== pinned.implementation) {
    throw new Error('SDK_PACKAGE_IDENTITY: installed official package differs from the pinned package');
  }
  const license = await fs.readFile(path.join(sdkRoot, resourceFiles[6]));
  if (hash(license) !== pinned.license) throw new Error('SDK_LICENSE_IDENTITY: exact upstream license required');
  const existingDestination = await fs.lstat(destination).catch(error => { if (error.code !== 'ENOENT') throw error; });
  if (existingDestination?.isSymbolicLink()) throw new Error('Packaging destination must not be a symlink');
  // Native macOS temp roots may legitimately traverse /var -> /private/var.
  // Resolve that ancestor once, then reject symlinks inside the bundle itself.
  await fs.mkdir(destination, { recursive: true });
  const target = await fs.realpath(destination);
  if (target === sdkRoot || sdkRoot.startsWith(target + path.sep)) throw new Error('Unsafe packaging destination');
  await verifyExistingFiles(target);
  const files = [];
  for (const name of resourceFiles) {
    const source = path.join(sdkRoot, name);
    const stat = await fs.lstat(source);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 2_097_152) throw new Error(`Unsafe resource: ${name}`);
    const bytes = await fs.readFile(source);
    const output = path.join(target, name);
    await safeDirectory(path.dirname(output), target);
    // A fresh staging directory is required. Never follow destination symlinks.
    const existing = await fs.lstat(output).catch(error => { if (error.code !== 'ENOENT') throw error; });
    if (existing?.isSymbolicLink()) throw new Error(`Unsafe staged resource: ${name}`);
    await fs.writeFile(output, bytes, { mode: 0o644 });
    files.push({ path: name, bytes: bytes.length, sha256: hash(bytes) });
  }
  const manifest = { schemaVersion: 'sdk-readiness-resources/v1', sdkPackage: metadata.name,
    sdkVersion: metadata.version, officialRuntimeIncluded: false, modelCalls: 0, files };
  await fs.writeFile(path.join(target, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
  return manifest;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 2) throw new Error('This build helper takes no caller-selected paths');
  const destination = path.join(sdkRoot, '../apps/desktop/src-tauri/resources/sdk-readiness');
  const manifest = await packageReadiness(destination);
  console.log(`Packaged ${manifest.files.length} verified SDK readiness resources; model/runtime/account calls: 0`);
}
