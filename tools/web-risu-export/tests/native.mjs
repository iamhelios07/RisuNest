import { mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { spawnSync, execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { gzipSync } from 'node:zlib';
import { exportBackup } from '../src/export.mjs';
import { MemorySink, coldId, database, fixtureSource } from './fixtures.mjs';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const output = path.join(root, '.tmp/native');
await mkdir(output, { recursive: true });
const db = database({ character: { coldstorage: coldId } });
const sink = new MemorySink();
await exportBackup({
    source: fixtureSource({
        db, assets: new Map([['assets/nested/image.webp', new Uint8Array([1, 2, 3])]]),
        cold: new Map([[coldId, gzipSync(Buffer.from('{"character":{"chaId":"character-1","chats":[]}}'))]]),
        inlays: new Map([['orphan-한글', { type: 'image', name: 'synthetic', ext: 'png', width: 1, height: 1, data: new Blob([Uint8Array.of(4, 5)]) }]])
    }),
    sink
});
const backup = path.join(output, 'synthetic.bin');
await writeFile(backup, sink.bytes);
const args = ['run', '--locked', '--offline', '--manifest-path', path.join(root, 'tests/native/Cargo.toml'), '--', backup, path.join(output, crypto.randomUUID()), createHash('sha256').update(db).digest('hex')];
const common = execFileSync('git', ['rev-parse', '--path-format=absolute', '--git-common-dir'], { cwd: root, encoding: 'utf8' }).trim();
const target = path.join(path.dirname(common), 'src-tauri/target');
if (process.env.CARGO_TARGET_DIR && path.resolve(process.env.CARGO_TARGET_DIR) !== path.resolve(target)) throw new Error('Use the repository shared Cargo target.');
const child = spawnSync('cargo', args, { stdio: 'inherit', cwd: root, env: { ...process.env, CARGO_TARGET_DIR: target } });
if (child.error) throw child.error;
process.exitCode = child.status ?? 1;
