import { sha256 } from '@noble/hashes/sha2.js';

export const CHUNK = 256 * 1024;
export const U32_MAX = 0xffffffff;
export const encoder = new TextEncoder();
export const decoder = new TextDecoder('utf-8', { fatal: true });
export class ExportError extends Error {
    constructor(code) { super(code); this.name = 'ExportError'; this.code = code; }
}
export function fail(code) { throw new ExportError(code); }
export function check(signal) { signal?.throwIfAborted(); }
export const hex = bytes => Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
export const keyHex = key => hex(encoder.encode(key));
export function fromHex(value) {
    if (!/^(?:[0-9a-f]{2})+$/i.test(value)) return null;
    try { return decoder.decode(Uint8Array.from(value.match(/../g), v => parseInt(v, 16))); }
    catch { return null; }
}
export function u32(value) {
    if (!Number.isSafeInteger(value) || value < 0 || value > U32_MAX) fail('entryTooLarge');
    const bytes = new Uint8Array(4);
    new DataView(bytes.buffer).setUint32(0, value, true);
    return bytes;
}
export function sourceOf(value) {
    if (value instanceof Blob) return { size: value.size, stream: () => value.stream() };
    const bytes = value instanceof ArrayBuffer ? new Uint8Array(value)
        : ArrayBuffer.isView(value) ? new Uint8Array(value.buffer, value.byteOffset, value.byteLength) : null;
    if (!bytes) fail('invalidData');
    return {
        size: bytes.byteLength,
        stream() {
            let offset = 0;
            return new ReadableStream({
                pull(controller) {
                    if (offset === bytes.length) { controller.close(); return; }
                    const end = Math.min(offset + CHUNK, bytes.length);
                    controller.enqueue(bytes.subarray(offset, end));
                    offset = end;
                }
            }, { highWaterMark: 0 });
        }
    };
}
export async function* chunks(stream, signal) {
    const reader = stream.getReader();
    const abort = () => { void reader.cancel(signal.reason).catch(() => {}); };
    signal?.addEventListener('abort', abort, { once: true });
    try {
        while (true) {
            check(signal);
            const { value, done } = await reader.read();
            check(signal);
            if (done) break;
            for (let offset = 0; offset < value.length; offset += CHUNK) {
                yield value.subarray(offset, Math.min(offset + CHUNK, value.length));
            }
        }
    } finally {
        signal?.removeEventListener('abort', abort);
        await reader.cancel().catch(() => {});
        reader.releaseLock();
    }
}
export function streamOf(iterable) {
    const iterator = iterable[Symbol.asyncIterator]();
    return new ReadableStream({
        async pull(controller) {
            try {
                const item = await iterator.next();
                if (item.done) controller.close(); else controller.enqueue(item.value);
            } catch (error) { controller.error(error); }
        },
        async cancel() { await iterator.return?.(); }
    }, { highWaterMark: 0 });
}
const yielders = new WeakMap();
const defaultYieldKey = {};
export function yielding(signal) {
    const key = signal ?? defaultYieldKey;
    if (yielders.has(key)) return yielders.get(key);
    let last = performance.now();
    const cooperate = async () => {
        check(signal);
        if (performance.now() - last < 12) return;
        await new Promise(resolve => setTimeout(resolve, 0));
        last = performance.now();
        check(signal);
    };
    // Nested stream stages share one clock so they do not alternate timer waits.
    yielders.set(key, cooperate);
    return cooperate;
}
export async function digest(source, signal, progress = () => {}) {
    const hash = sha256.create();
    const breathe = yielding(signal);
    let size = 0;
    for await (const bytes of chunks(source.stream(), signal)) {
        hash.update(bytes); size += bytes.length; progress(bytes.length);
        await breathe();
    }
    if (source.size != null && size !== source.size) fail('changed');
    return hex(hash.digest());
}
export class ByteReader {
    constructor(stream, signal, onChunk = async () => {}) {
        this.iterator = chunks(stream, signal)[Symbol.asyncIterator]();
        this.signal = signal; this.onChunk = onChunk;
        this.buffer = new Uint8Array(); this.offset = 0; this.read = 0;
    }
    async available() {
        check(this.signal);
        while (this.offset === this.buffer.length) {
            const item = await this.iterator.next();
            if (item.done) return false;
            this.buffer = item.value; this.offset = 0;
            await this.onChunk(item.value);
        }
        return true;
    }
    async take(length) {
        if (!Number.isSafeInteger(length) || length < 0) fail('invalidData');
        if (length === 0) return new Uint8Array();
        if (!await this.available()) fail('truncated');
        if (this.buffer.length - this.offset >= length) {
            const result = this.buffer.subarray(this.offset, this.offset + length);
            this.offset += length; this.read += length; return result;
        }
        const result = new Uint8Array(length);
        let offset = 0;
        for await (const part of this.parts(length)) { result.set(part, offset); offset += part.length; }
        return result;
    }
    async* parts(length) {
        while (length > 0) {
            if (!await this.available()) fail('truncated');
            const size = Math.min(length, this.buffer.length - this.offset);
            const part = this.buffer.subarray(this.offset, this.offset + size);
            this.offset += size; this.read += size; length -= size;
            yield part;
        }
    }
    async* rest() {
        while (await this.available()) {
            const part = this.buffer.subarray(this.offset);
            this.offset = this.buffer.length; this.read += part.length;
            yield part;
        }
    }
    async number(length, little = false) {
        const bytes = await this.take(length);
        const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
        if (length === 1) return bytes[0];
        return length === 2 ? view.getUint16(0, little) : view.getUint32(0, little);
    }
    async close() { await this.iterator.return?.(); this.buffer = new Uint8Array(); }
}
export function archiveName(name) {
    if (typeof name !== 'string' || !name || /[\\\u0000-\u001f:]/.test(name)
        || name.startsWith('/') || name.split('/').some(p => !p || p === '.' || p === '..')) fail('invalidName');
    const bytes = encoder.encode(name);
    if (bytes.length > 1024 * 1024) fail('invalidName');
    return bytes;
}
export class ArchiveWriter {
    constructor(sink, signal, onBytes = () => {}, bufferSize = 1024 * 1024) {
        this.sink = sink; this.signal = signal; this.onBytes = onBytes;
        this.buffer = new Uint8Array(bufferSize); this.used = 0;
        this.position = 0; this.names = new Set(); this.nameBytes = 0; this.active = null;
    }
    async flush() {
        check(this.signal);
        if (!this.used) return;
        await this.sink.write(this.buffer.subarray(0, this.used));
        this.used = 0;
    }
    async raw(bytes) {
        check(this.signal);
        let offset = 0;
        while (offset < bytes.length) {
            const amount = Math.min(this.buffer.length - this.used, bytes.length - offset);
            this.buffer.set(bytes.subarray(offset, offset + amount), this.used);
            this.used += amount; offset += amount; this.position += amount;
            this.onBytes(amount);
            if (this.used === this.buffer.length) await this.flush();
        }
    }
    async begin(name, size = null) {
        if (this.active) fail('invalidData');
        const encoded = archiveName(name);
        if (this.names.has(name)) fail('duplicate');
        if (this.names.size >= 200000 || this.nameBytes + encoded.length > 64 * 1024 * 1024) fail('tooMany');
        if (size != null) u32(size);
        this.names.add(name); this.nameBytes += encoded.length;
        await this.raw(u32(encoded.length)); await this.raw(encoded);
        const lengthPosition = this.position;
        await this.raw(u32(size ?? 0));
        this.active = { start: this.position, lengthPosition, expected: size };
    }
    async append(bytes) {
        if (!this.active || this.position - this.active.start + bytes.length > U32_MAX) fail('entryTooLarge');
        await this.raw(bytes);
    }
    async end() {
        const entry = this.active;
        if (!entry) fail('invalidData');
        const size = this.position - entry.start;
        if (entry.expected != null && size !== entry.expected) fail('changed');
        if (entry.expected == null) {
            await this.flush();
            await this.sink.seek(entry.lengthPosition);
            await this.sink.write(u32(size));
            await this.sink.seek(this.position);
        }
        this.active = null;
        return size;
    }
    async entry(name, source) {
        await this.begin(name, source.size);
        for await (const part of chunks(source.stream(), this.signal)) await this.append(part);
        return this.end();
    }
    async finish() {
        if (this.active || !this.names.has('database.risudat')) fail('invalidData');
        await this.flush(); check(this.signal); await this.sink.close();
    }
    async abort() { this.used = 0; await this.sink.abort().catch(() => {}); }
}
