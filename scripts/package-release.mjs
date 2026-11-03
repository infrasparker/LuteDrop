// Stages every GitHub release asset into release/ (git-ignored).
// Run `cargo tauri build` first, then `npm run package:release`.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const at = (...p) => path.join(root, ...p);
const fail = (msg) => { console.error(`package-release: ${msg}`); process.exit(1); };

// --- 1. Make sure every place that stores the version agrees. ---------------
const V = JSON.parse(fs.readFileSync(at('package.json'), 'utf8')).version;
const moduleDir = at('foundry-module', 'lutedrop-stream');
const moduleJson = JSON.parse(fs.readFileSync(path.join(moduleDir, 'module.json'), 'utf8'));
const tauriVersion = JSON.parse(fs.readFileSync(at('src-tauri', 'tauri.conf.json'), 'utf8')).version;
// Cargo.toml: the first `version = "..."` line is the [package] one.
const cargoVersion = fs.readFileSync(at('src-tauri', 'Cargo.toml'), 'utf8').match(/^version\s*=\s*"([^"]+)"/m)?.[1];

const mismatches = [
  ['src-tauri/tauri.conf.json', tauriVersion],
  ['src-tauri/Cargo.toml', cargoVersion],
  ['module.json', moduleJson.version],
].filter(([, v]) => v !== V);
if (mismatches.length) {
  fail(`version mismatch (package.json is ${V}): ${mismatches.map(([n, v]) => `${n} is ${v}`).join(', ')}`);
}
if (!moduleJson.download?.includes(`/v${V}/`)) {
  fail(`module.json "download" must contain /v${V}/ (got ${moduleJson.download})`);
}

// --- 2. Check the build outputs exist before touching release/. -------------
const nsis = at('src-tauri', 'target', 'release', 'bundle', 'nsis', `LuteDrop_${V}_x64-setup.exe`);
const msi = at('src-tauri', 'target', 'release', 'bundle', 'msi', `LuteDrop_${V}_x64_en-US.msi`);
const exe = at('src-tauri', 'target', 'release', 'lutedrop.exe');
const remote = at('dist-remote');
if (![nsis, msi].every(fs.existsSync)) fail('installers for this version are missing: run `cargo tauri build` first');
if (!fs.existsSync(exe)) fail('src-tauri/target/release/lutedrop.exe is missing: run `cargo tauri build` first');
if (!fs.existsSync(remote)) fail('dist-remote/ is missing: run `npm run build:remote` (cargo tauri build does this too)');

// --- 3. Start with an empty release/ folder. --------------------------------
const out = at('release');
fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(out);

// Zips `entries` (relative to `dir`) into `zipPath`. We avoid PowerShell's
// Compress-Archive because Windows PowerShell 5.1 writes backslashes in entry
// names, which breaks unpacking on Linux-hosted Foundry servers. Windows' built-in
// bsdtar writes proper forward slashes (-a picks the zip format from ".zip").
function zip(zipPath, dir, entries) {
  try {
    zipUnchecked(zipPath, dir, entries);
  } catch (e) {
    throw new Error(`zipping ${path.basename(zipPath)} failed: ${e.message}`);
  }
}
function zipUnchecked(zipPath, dir, entries) {
  if (process.platform === 'win32') {
    execFileSync(path.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe'), ['-a', '-cf', zipPath, '-C', dir, ...entries], { stdio: 'inherit' });
  } else {
    execFileSync('zip', ['-r', zipPath, ...entries], { cwd: dir, stdio: 'inherit' });
  }
}

// --- 4. Stage the assets. ----------------------------------------------------
fs.copyFileSync(nsis, path.join(out, path.basename(nsis)));
fs.copyFileSync(msi, path.join(out, path.basename(msi)));

// Portable: the exe plus dist-remote/ (which the phone remote needs) side by side.
const stage = fs.mkdtempSync(path.join(out, '.portable-'));
try {
  fs.copyFileSync(exe, path.join(stage, 'lutedrop.exe'));
  fs.cpSync(remote, path.join(stage, 'dist-remote'), { recursive: true });
  zip(path.join(out, `LuteDrop_${V}_x64_portable.zip`), stage, ['lutedrop.exe', 'dist-remote']);
} catch (e) {
  fs.rmSync(stage, { recursive: true, force: true }); // don't leave the temp folder behind
  fail(e.message);
}
fs.rmSync(stage, { recursive: true, force: true });

// Foundry module: module.json must sit at the zip root, not inside a folder.
// Only these entries ship; anything else in the folder (Thumbs.db, editor backups) is an error.
const moduleEntries = ['module.json', 'lang', 'scripts', 'styles'];
const stray = fs.readdirSync(moduleDir).filter((n) => !moduleEntries.includes(n));
if (stray.length) fail(`unexpected entries in foundry-module/lutedrop-stream: ${stray.join(', ')} (remove them)`);
try {
  zip(path.join(out, 'lutedrop-stream.zip'), moduleDir, moduleEntries);
} catch (e) {
  fail(e.message);
}
fs.copyFileSync(path.join(moduleDir, 'module.json'), path.join(out, 'module.json'));

// --- 5. Tell the user what to do next. ---------------------------------------
console.log('\nStaged in release/:');
for (const f of fs.readdirSync(out)) console.log(`  ${f}`);
// Files are listed one by one because PowerShell does not expand wildcards for gh.
const assets = [`LuteDrop_${V}_x64-setup.exe`, `LuteDrop_${V}_x64_en-US.msi`, `LuteDrop_${V}_x64_portable.zip`, 'lutedrop-stream.zip', 'module.json'];
console.log(`\nNext: gh release create v${V} ${assets.map((f) => `release/${f}`).join(' ')} --title "LuteDrop v${V}" --notes-file <notes>`);
