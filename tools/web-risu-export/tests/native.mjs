import { mkdir, readFile, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync, execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { gzipSync } from 'node:zlib';
import { exportBackup } from '../src/export.mjs';
import { COLD_HEADER } from '../src/scan.mjs';
import { MemorySink, coldId, database, fixtureSource, rawBlock, readBlocks } from './fixtures.mjs';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const output = path.join(root, '.tmp/native');
await mkdir(output, { recursive: true });
// A chat-shaped cold payload is restored; a referenced settings payload is skipped and restored as unavailable.
const skippedCold = '30000000-0000-4000-8000-000000000003';
const character = { image: 'assets/missing.png', coldstorage: coldId, chats: [{ name: 'Synthetic chat', message: [{ data: COLD_HEADER + skippedCold }] }] };
// Cold-stored plugin values, including falsy primitives, are inlined; the SaveKV block is copied.
const [pluginIndex, pluginLarge] = ['30000000-0000-4000-8000-000000000004', '30000000-0000-4000-8000-000000000005'];
const large = { text: '큰 값'.repeat(1024) };
const plugins = { null: null, false: false, zero: 0, empty: '', large, inline: 'block value' };
const index = { type: 'risu-plugin-storage-index', version: 1, trash: [], entries: [
    { k: 'null', s: 'null' }, { k: 'false', s: 'false' }, { k: 'zero', s: '0' }, { k: 'empty', s: '""' }, { k: 'large', l: pluginLarge }
] };
const db = database({ character, pluginStorage: { _coldpluginIndex: pluginIndex, inline: 'block value' }, blocks: [[12, 'kv:synthetic', { version: 1, entries: [{ k: 'key', v: 'value' }] }]] });
const sink = new MemorySink();
const result = await exportBackup({
    source: fixtureSource({
        db, assets: new Map([['assets/nested/image.webp', new Uint8Array([1, 2, 3])]]),
        cold: new Map([
            [coldId, gzipSync(Buffer.from(JSON.stringify({ character: { chaId: 'character-1', name: 'Synthetic', chats: character.chats } })))],
            [skippedCold, gzipSync(Buffer.from('{"settings":true}'))],
            [pluginIndex, gzipSync(JSON.stringify(index))], [pluginLarge, gzipSync(JSON.stringify(large))]
        ]),
        inlays: new Map([
            ['orphan-한글', { type: 'image', name: 'synthetic', ext: 'png', width: 1, height: 1, data: new Blob([Uint8Array.of(4, 5)]) }],
            ['legacy-untyped', { name: 'legacy', ext: 'png', data: 'data:image/png;base64,' + Buffer.from([6, 7]).toString('base64') }]
        ])
    }),
    sink
});
assert.deepEqual(result.skipped, { missingAssets: 1, missingCold: 0, unreadableCold: 1, pluginCold: 0, unsupportedInlays: 0 });
const backup = path.join(output, 'synthetic.bin');
await writeFile(backup, sink.bytes);
const common = execFileSync('git', ['rev-parse', '--path-format=absolute', '--git-common-dir'], { cwd: root, encoding: 'utf8' }).trim();
const target = path.join(path.dirname(common), 'src-tauri/target');
if (process.env.CARGO_TARGET_DIR && path.resolve(process.env.CARGO_TARGET_DIR) !== path.resolve(target)) throw new Error('Use the repository shared Cargo target.');
const cargo = (args, env = {}) => {
    const child = spawnSync('cargo', args, { stdio: 'inherit', cwd: root, env: { ...process.env, ...env, CARGO_TARGET_DIR: target } });
    if (child.error) throw child.error;
    if (child.status !== 0) process.exit(child.status ?? 1);
};
// The written database carries the rewritten pluginStorage block last.
const written = Buffer.concat([Buffer.from('RISUSAVE\0'), ...readBlocks(db).filter(b => b.name !== 'pluginStorage').map(b => b.bytes),
    rawBlock(11, 'pluginStorage', Buffer.from(JSON.stringify(plugins)))]);
cargo(['run', '--locked', '--offline', '--manifest-path', path.join(root, 'tests/native/Cargo.toml'), '--', backup, path.join(output, crypto.randomUUID()), createHash('sha256').update(written).digest('hex')]);
// Restores the archive through RisuNest's legacy backup path, including cold and Inlay preparation.
const report = path.join(output, crypto.randomUUID() + '.json');
cargo(['test', '--lib', '--locked', '--offline', '--manifest-path', path.join(root, '../../src-tauri/Cargo.toml'),
    'native_file_jobs::restore::tests::web_export_archive_restores_through_the_legacy_backup_path', '--', '--ignored', '--exact'],
    { RISUNEST_WEB_EXPORT_ARCHIVE: backup, RISUNEST_WEB_EXPORT_REPORT: report });
const restored = JSON.parse(await readFile(report, 'utf8'));
assert.equal(restored.characterCount, 1);
assert.deepEqual(restored.root.pluginCustomStorage, plugins);
assert.deepEqual(restored.payloads.coldPayloads, [coldId]);
assert.deepEqual(restored.payloads.invalidInlays, []);
for (const key of ['orphan-한글', 'legacy-untyped']) {
    assert(restored.payloads.aliases.some(alias => alias.kind === 'inlay' && alias.key === key && alias.inlayType === 'image'), key);
}
assert(restored.payloads.aliases.some(alias => alias.key === 'assets/nested/image.webp'));
assert.deepEqual(restored.unavailableCold, [skippedCold]);
console.log('Restored the synthetic web export through the RisuNest legacy backup path.');
