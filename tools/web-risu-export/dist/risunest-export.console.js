// Third-party licenses: THIRD_PARTY_LICENSES.md
(() => {
  // node_modules/@noble/hashes/utils.js
  function isBytes(a) {
    return a instanceof Uint8Array || ArrayBuffer.isView(a) && a.constructor.name === "Uint8Array";
  }
  function abytes(value, length, title = "") {
    const bytes = isBytes(value);
    const len = value?.length;
    const needsLen = length !== void 0;
    if (!bytes || needsLen && len !== length) {
      const prefix = title && `"${title}" `;
      const ofLen = needsLen ? ` of length ${length}` : "";
      const got = bytes ? `length=${len}` : `type=${typeof value}`;
      throw new Error(prefix + "expected Uint8Array" + ofLen + ", got " + got);
    }
    return value;
  }
  function aexists(instance, checkFinished = true) {
    if (instance.destroyed)
      throw new Error("Hash instance has been destroyed");
    if (checkFinished && instance.finished)
      throw new Error("Hash#digest() has already been called");
  }
  function aoutput(out, instance) {
    abytes(out, void 0, "digestInto() output");
    const min = instance.outputLen;
    if (out.length < min) {
      throw new Error('"digestInto() output" expected to be of length >=' + min);
    }
  }
  function clean(...arrays) {
    for (let i = 0; i < arrays.length; i++) {
      arrays[i].fill(0);
    }
  }
  function createView(arr) {
    return new DataView(arr.buffer, arr.byteOffset, arr.byteLength);
  }
  function rotr(word, shift) {
    return word << 32 - shift | word >>> shift;
  }
  function createHasher(hashCons, info = {}) {
    const hashC = (msg, opts) => hashCons(opts).update(msg).digest();
    const tmp = hashCons(void 0);
    hashC.outputLen = tmp.outputLen;
    hashC.blockLen = tmp.blockLen;
    hashC.create = (opts) => hashCons(opts);
    Object.assign(hashC, info);
    return Object.freeze(hashC);
  }
  var oidNist = (suffix) => ({
    oid: Uint8Array.from([6, 9, 96, 134, 72, 1, 101, 3, 4, 2, suffix])
  });

  // node_modules/@noble/hashes/_md.js
  function Chi(a, b, c) {
    return a & b ^ ~a & c;
  }
  function Maj(a, b, c) {
    return a & b ^ a & c ^ b & c;
  }
  var HashMD = class {
    blockLen;
    outputLen;
    padOffset;
    isLE;
    // For partial updates less than block size
    buffer;
    view;
    finished = false;
    length = 0;
    pos = 0;
    destroyed = false;
    constructor(blockLen, outputLen, padOffset, isLE) {
      this.blockLen = blockLen;
      this.outputLen = outputLen;
      this.padOffset = padOffset;
      this.isLE = isLE;
      this.buffer = new Uint8Array(blockLen);
      this.view = createView(this.buffer);
    }
    update(data) {
      aexists(this);
      abytes(data);
      const { view, buffer, blockLen } = this;
      const len = data.length;
      for (let pos = 0; pos < len; ) {
        const take = Math.min(blockLen - this.pos, len - pos);
        if (take === blockLen) {
          const dataView = createView(data);
          for (; blockLen <= len - pos; pos += blockLen)
            this.process(dataView, pos);
          continue;
        }
        buffer.set(data.subarray(pos, pos + take), this.pos);
        this.pos += take;
        pos += take;
        if (this.pos === blockLen) {
          this.process(view, 0);
          this.pos = 0;
        }
      }
      this.length += data.length;
      this.roundClean();
      return this;
    }
    digestInto(out) {
      aexists(this);
      aoutput(out, this);
      this.finished = true;
      const { buffer, view, blockLen, isLE } = this;
      let { pos } = this;
      buffer[pos++] = 128;
      clean(this.buffer.subarray(pos));
      if (this.padOffset > blockLen - pos) {
        this.process(view, 0);
        pos = 0;
      }
      for (let i = pos; i < blockLen; i++)
        buffer[i] = 0;
      view.setBigUint64(blockLen - 8, BigInt(this.length * 8), isLE);
      this.process(view, 0);
      const oview = createView(out);
      const len = this.outputLen;
      if (len % 4)
        throw new Error("_sha2: outputLen must be aligned to 32bit");
      const outLen = len / 4;
      const state = this.get();
      if (outLen > state.length)
        throw new Error("_sha2: outputLen bigger than state");
      for (let i = 0; i < outLen; i++)
        oview.setUint32(4 * i, state[i], isLE);
    }
    digest() {
      const { buffer, outputLen } = this;
      this.digestInto(buffer);
      const res = buffer.slice(0, outputLen);
      this.destroy();
      return res;
    }
    _cloneInto(to) {
      to ||= new this.constructor();
      to.set(...this.get());
      const { blockLen, buffer, length, finished, destroyed, pos } = this;
      to.destroyed = destroyed;
      to.finished = finished;
      to.length = length;
      to.pos = pos;
      if (length % blockLen)
        to.buffer.set(buffer);
      return to;
    }
    clone() {
      return this._cloneInto();
    }
  };
  var SHA256_IV = /* @__PURE__ */ Uint32Array.from([
    1779033703,
    3144134277,
    1013904242,
    2773480762,
    1359893119,
    2600822924,
    528734635,
    1541459225
  ]);

  // node_modules/@noble/hashes/sha2.js
  var SHA256_K = /* @__PURE__ */ Uint32Array.from([
    1116352408,
    1899447441,
    3049323471,
    3921009573,
    961987163,
    1508970993,
    2453635748,
    2870763221,
    3624381080,
    310598401,
    607225278,
    1426881987,
    1925078388,
    2162078206,
    2614888103,
    3248222580,
    3835390401,
    4022224774,
    264347078,
    604807628,
    770255983,
    1249150122,
    1555081692,
    1996064986,
    2554220882,
    2821834349,
    2952996808,
    3210313671,
    3336571891,
    3584528711,
    113926993,
    338241895,
    666307205,
    773529912,
    1294757372,
    1396182291,
    1695183700,
    1986661051,
    2177026350,
    2456956037,
    2730485921,
    2820302411,
    3259730800,
    3345764771,
    3516065817,
    3600352804,
    4094571909,
    275423344,
    430227734,
    506948616,
    659060556,
    883997877,
    958139571,
    1322822218,
    1537002063,
    1747873779,
    1955562222,
    2024104815,
    2227730452,
    2361852424,
    2428436474,
    2756734187,
    3204031479,
    3329325298
  ]);
  var SHA256_W = /* @__PURE__ */ new Uint32Array(64);
  var SHA2_32B = class extends HashMD {
    constructor(outputLen) {
      super(64, outputLen, 8, false);
    }
    get() {
      const { A, B, C, D, E, F, G, H } = this;
      return [A, B, C, D, E, F, G, H];
    }
    // prettier-ignore
    set(A, B, C, D, E, F, G, H) {
      this.A = A | 0;
      this.B = B | 0;
      this.C = C | 0;
      this.D = D | 0;
      this.E = E | 0;
      this.F = F | 0;
      this.G = G | 0;
      this.H = H | 0;
    }
    process(view, offset) {
      for (let i = 0; i < 16; i++, offset += 4)
        SHA256_W[i] = view.getUint32(offset, false);
      for (let i = 16; i < 64; i++) {
        const W15 = SHA256_W[i - 15];
        const W2 = SHA256_W[i - 2];
        const s0 = rotr(W15, 7) ^ rotr(W15, 18) ^ W15 >>> 3;
        const s1 = rotr(W2, 17) ^ rotr(W2, 19) ^ W2 >>> 10;
        SHA256_W[i] = s1 + SHA256_W[i - 7] + s0 + SHA256_W[i - 16] | 0;
      }
      let { A, B, C, D, E, F, G, H } = this;
      for (let i = 0; i < 64; i++) {
        const sigma1 = rotr(E, 6) ^ rotr(E, 11) ^ rotr(E, 25);
        const T1 = H + sigma1 + Chi(E, F, G) + SHA256_K[i] + SHA256_W[i] | 0;
        const sigma0 = rotr(A, 2) ^ rotr(A, 13) ^ rotr(A, 22);
        const T2 = sigma0 + Maj(A, B, C) | 0;
        H = G;
        G = F;
        F = E;
        E = D + T1 | 0;
        D = C;
        C = B;
        B = A;
        A = T1 + T2 | 0;
      }
      A = A + this.A | 0;
      B = B + this.B | 0;
      C = C + this.C | 0;
      D = D + this.D | 0;
      E = E + this.E | 0;
      F = F + this.F | 0;
      G = G + this.G | 0;
      H = H + this.H | 0;
      this.set(A, B, C, D, E, F, G, H);
    }
    roundClean() {
      clean(SHA256_W);
    }
    destroy() {
      this.set(0, 0, 0, 0, 0, 0, 0, 0);
      clean(this.buffer);
    }
  };
  var _SHA256 = class extends SHA2_32B {
    // We cannot use array here since array allows indexing by variable
    // which means optimizer/compiler cannot use registers.
    A = SHA256_IV[0] | 0;
    B = SHA256_IV[1] | 0;
    C = SHA256_IV[2] | 0;
    D = SHA256_IV[3] | 0;
    E = SHA256_IV[4] | 0;
    F = SHA256_IV[5] | 0;
    G = SHA256_IV[6] | 0;
    H = SHA256_IV[7] | 0;
    constructor() {
      super(32);
    }
  };
  var sha256 = /* @__PURE__ */ createHasher(
    () => new _SHA256(),
    /* @__PURE__ */ oidNist(1)
  );

  // src/bytes.mjs
  var CHUNK = 256 * 1024;
  var U32_MAX = 4294967295;
  var encoder = new TextEncoder();
  var decoder = new TextDecoder("utf-8", { fatal: true });
  var ExportError = class extends Error {
    constructor(code) {
      super(code);
      this.name = "ExportError";
      this.code = code;
    }
  };
  function fail(code) {
    throw new ExportError(code);
  }
  function check(signal3) {
    signal3?.throwIfAborted();
  }
  var wait = (milliseconds, signal3) => new Promise((resolve, reject) => {
    check(signal3);
    const abort = () => {
      clearTimeout(timer);
      reject(signal3.reason);
    };
    const timer = setTimeout(() => {
      signal3?.removeEventListener("abort", abort);
      resolve();
    }, milliseconds);
    signal3?.addEventListener("abort", abort, { once: true });
  });
  var hex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  var keyHex = (key) => hex(encoder.encode(key));
  function fromHex(value) {
    if (!/^(?:[0-9a-f]{2})+$/i.test(value)) return null;
    try {
      return decoder.decode(Uint8Array.from(value.match(/../g), (v) => parseInt(v, 16)));
    } catch {
      return null;
    }
  }
  function u32(value) {
    if (!Number.isSafeInteger(value) || value < 0 || value > U32_MAX) fail("entryTooLarge");
    const bytes = new Uint8Array(4);
    new DataView(bytes.buffer).setUint32(0, value, true);
    return bytes;
  }
  function sourceOf(value) {
    if (value instanceof Blob) return { size: value.size, stream: () => value.stream() };
    const bytes = value instanceof ArrayBuffer ? new Uint8Array(value) : ArrayBuffer.isView(value) ? new Uint8Array(value.buffer, value.byteOffset, value.byteLength) : null;
    if (!bytes) fail("invalidData");
    return {
      size: bytes.byteLength,
      stream() {
        let offset = 0;
        return new ReadableStream({
          pull(controller) {
            if (offset === bytes.length) {
              controller.close();
              return;
            }
            const end = Math.min(offset + CHUNK, bytes.length);
            controller.enqueue(bytes.subarray(offset, end));
            offset = end;
          }
        }, { highWaterMark: 0 });
      }
    };
  }
  async function* chunks(stream, signal3) {
    const reader = stream.getReader();
    const abort = () => {
      void reader.cancel(signal3.reason).catch(() => {
      });
    };
    signal3?.addEventListener("abort", abort, { once: true });
    try {
      while (true) {
        check(signal3);
        const { value, done } = await reader.read();
        check(signal3);
        if (done) break;
        for (let offset = 0; offset < value.length; offset += CHUNK) {
          yield value.subarray(offset, Math.min(offset + CHUNK, value.length));
        }
      }
    } finally {
      signal3?.removeEventListener("abort", abort);
      await reader.cancel().catch(() => {
      });
      reader.releaseLock();
    }
  }
  function streamOf(iterable) {
    const iterator = iterable[Symbol.asyncIterator]();
    return new ReadableStream({
      async pull(controller) {
        try {
          const item = await iterator.next();
          if (item.done) controller.close();
          else controller.enqueue(item.value);
        } catch (error) {
          controller.error(error);
        }
      },
      async cancel() {
        await iterator.return?.();
      }
    }, { highWaterMark: 0 });
  }
  var yielders = /* @__PURE__ */ new WeakMap();
  var defaultYieldKey = {};
  function yielding(signal3) {
    const key = signal3 ?? defaultYieldKey;
    if (yielders.has(key)) return yielders.get(key);
    let last = performance.now();
    const cooperate = async () => {
      check(signal3);
      if (performance.now() - last < 12) return;
      await new Promise((resolve) => setTimeout(resolve, 0));
      last = performance.now();
      check(signal3);
    };
    yielders.set(key, cooperate);
    return cooperate;
  }
  async function digest(source, signal3, progress = () => {
  }) {
    const hash = sha256.create();
    const breathe = yielding(signal3);
    let size = 0;
    for await (const bytes of chunks(source.stream(), signal3)) {
      hash.update(bytes);
      size += bytes.length;
      progress(bytes.length);
      await breathe();
    }
    if (source.size != null && size !== source.size) fail("changed");
    return hex(hash.digest());
  }
  var ByteReader = class {
    constructor(stream, signal3, onChunk = async () => {
    }) {
      this.iterator = chunks(stream, signal3)[Symbol.asyncIterator]();
      this.signal = signal3;
      this.onChunk = onChunk;
      this.buffer = new Uint8Array();
      this.offset = 0;
      this.read = 0;
    }
    async available() {
      check(this.signal);
      while (this.offset === this.buffer.length) {
        const item = await this.iterator.next();
        if (item.done) return false;
        this.buffer = item.value;
        this.offset = 0;
        await this.onChunk(item.value);
      }
      return true;
    }
    async take(length) {
      if (!Number.isSafeInteger(length) || length < 0) fail("invalidData");
      if (length === 0) return new Uint8Array();
      if (!await this.available()) fail("truncated");
      if (this.buffer.length - this.offset >= length) {
        const result2 = this.buffer.subarray(this.offset, this.offset + length);
        this.offset += length;
        this.read += length;
        return result2;
      }
      const result = new Uint8Array(length);
      let offset = 0;
      for await (const part of this.parts(length)) {
        result.set(part, offset);
        offset += part.length;
      }
      return result;
    }
    async *parts(length) {
      while (length > 0) {
        if (!await this.available()) fail("truncated");
        const size = Math.min(length, this.buffer.length - this.offset);
        const part = this.buffer.subarray(this.offset, this.offset + size);
        this.offset += size;
        this.read += size;
        length -= size;
        yield part;
      }
    }
    async *rest() {
      while (await this.available()) {
        const part = this.buffer.subarray(this.offset);
        this.offset = this.buffer.length;
        this.read += part.length;
        yield part;
      }
    }
    async number(length, little = false) {
      const bytes = await this.take(length);
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
      if (length === 1) return bytes[0];
      return length === 2 ? view.getUint16(0, little) : view.getUint32(0, little);
    }
    async close() {
      await this.iterator.return?.();
      this.buffer = new Uint8Array();
    }
  };
  function archiveName(name) {
    if (typeof name !== "string" || !name || /[\\\u0000-\u001f:]/.test(name) || name.startsWith("/") || name.split("/").some((p) => !p || p === "." || p === "..")) fail("invalidName");
    const bytes = encoder.encode(name);
    if (bytes.length > 1024 * 1024) fail("invalidName");
    return bytes;
  }
  var ArchiveWriter = class {
    constructor(sink, signal3, onBytes = () => {
    }, bufferSize = 1024 * 1024) {
      this.sink = sink;
      this.signal = signal3;
      this.onBytes = onBytes;
      this.buffer = new Uint8Array(bufferSize);
      this.used = 0;
      this.position = 0;
      this.extent = 0;
      this.names = /* @__PURE__ */ new Set();
      this.nameBytes = 0;
      this.active = null;
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
        this.used += amount;
        offset += amount;
        this.position += amount;
        this.onBytes(amount);
        if (this.used === this.buffer.length) await this.flush();
      }
    }
    async begin(name, size = null) {
      if (this.active) fail("invalidData");
      const encoded = archiveName(name);
      if (this.names.has(name)) fail("duplicate");
      if (size != null) u32(size);
      this.names.add(name);
      this.nameBytes += encoded.length;
      const origin = this.position;
      await this.raw(u32(encoded.length));
      await this.raw(encoded);
      const lengthPosition = this.position;
      await this.raw(u32(size ?? 0));
      this.active = { name, nameBytes: encoded.length, origin, start: this.position, lengthPosition, expected: size };
    }
    // Discards the unfinished entry; later entries overwrite its bytes and finish() trims any excess.
    async rollback() {
      const entry = this.active;
      if (!entry) return;
      this.active = null;
      this.names.delete(entry.name);
      this.nameBytes -= entry.nameBytes;
      const flushed = this.position - this.used;
      if (entry.origin >= flushed) this.used -= this.position - entry.origin;
      else {
        this.used = 0;
        this.extent = Math.max(this.extent, flushed);
        await this.sink.seek(entry.origin);
      }
      this.onBytes(entry.origin - this.position);
      this.position = entry.origin;
    }
    async append(bytes) {
      if (!this.active || this.position - this.active.start + bytes.length > U32_MAX) fail("entryTooLarge");
      await this.raw(bytes);
    }
    async end() {
      const entry = this.active;
      if (!entry) fail("invalidData");
      const size = this.position - entry.start;
      if (entry.expected != null && size !== entry.expected) fail("changed");
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
      if (this.active || !this.names.has("database.risudat")) fail("invalidData");
      await this.flush();
      check(this.signal);
      if (this.extent > this.position) await this.sink.truncate(this.position);
      await this.sink.close();
    }
    async abort() {
      this.used = 0;
      await this.sink.abort().catch(() => {
      });
    }
  };

  // node_modules/oauth4webapi/build/index.js
  var USER_AGENT;
  if (typeof navigator === "undefined" || !navigator.userAgent?.startsWith?.("Mozilla/5.0 ")) {
    const NAME = "oauth4webapi";
    const VERSION = "v3.8.8";
    USER_AGENT = `${NAME}/${VERSION}`;
  }
  function looseInstanceOf(input, expected) {
    if (input == null) {
      return false;
    }
    try {
      return input instanceof expected || Object.getPrototypeOf(input)[Symbol.toStringTag] === expected.prototype[Symbol.toStringTag];
    } catch {
      return false;
    }
  }
  var ERR_INVALID_ARG_VALUE = "ERR_INVALID_ARG_VALUE";
  var ERR_INVALID_ARG_TYPE = "ERR_INVALID_ARG_TYPE";
  function CodedTypeError(message, code, cause) {
    const err = new TypeError(message, { cause });
    Object.assign(err, { code });
    return err;
  }
  var allowInsecureRequests = Symbol();
  var clockSkew = Symbol();
  var clockTolerance = Symbol();
  var customFetch = Symbol();
  var modifyAssertion = Symbol();
  var jweDecrypt = Symbol();
  var jwksCache = Symbol();
  var encoder2 = new TextEncoder();
  var decoder2 = new TextDecoder();
  function buf(input) {
    if (typeof input === "string") {
      return encoder2.encode(input);
    }
    return decoder2.decode(input);
  }
  var encodeBase64Url;
  if (Uint8Array.prototype.toBase64) {
    encodeBase64Url = (input) => {
      if (input instanceof ArrayBuffer) {
        input = new Uint8Array(input);
      }
      return input.toBase64({ alphabet: "base64url", omitPadding: true });
    };
  } else {
    const CHUNK_SIZE = 32768;
    encodeBase64Url = (input) => {
      if (input instanceof ArrayBuffer) {
        input = new Uint8Array(input);
      }
      const arr = [];
      for (let i = 0; i < input.byteLength; i += CHUNK_SIZE) {
        arr.push(String.fromCharCode.apply(null, input.subarray(i, i + CHUNK_SIZE)));
      }
      return btoa(arr.join("")).replace(/=/g, "").replace(/\+/g, "-").replace(/\//g, "_");
    };
  }
  var decodeBase64Url;
  if (Uint8Array.fromBase64) {
    decodeBase64Url = (input) => {
      try {
        return Uint8Array.fromBase64(input, { alphabet: "base64url" });
      } catch (cause) {
        throw CodedTypeError("The input to be decoded is not correctly encoded.", ERR_INVALID_ARG_VALUE, cause);
      }
    };
  } else {
    decodeBase64Url = (input) => {
      try {
        const binary = atob(input.replace(/-/g, "+").replace(/_/g, "/").replace(/\s/g, ""));
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) {
          bytes[i] = binary.charCodeAt(i);
        }
        return bytes;
      } catch (cause) {
        throw CodedTypeError("The input to be decoded is not correctly encoded.", ERR_INVALID_ARG_VALUE, cause);
      }
    };
  }
  function b64u(input) {
    if (typeof input === "string") {
      return decodeBase64Url(input);
    }
    return encodeBase64Url(input);
  }
  var UnsupportedOperationError = class extends Error {
    code;
    constructor(message, options) {
      super(message, options);
      this.name = this.constructor.name;
      this.code = UNSUPPORTED_OPERATION;
      Error.captureStackTrace?.(this, this.constructor);
    }
  };
  var OperationProcessingError = class extends Error {
    code;
    constructor(message, options) {
      super(message, options);
      this.name = this.constructor.name;
      if (options?.code) {
        this.code = options?.code;
      }
      Error.captureStackTrace?.(this, this.constructor);
    }
  };
  function OPE(message, code, cause) {
    return new OperationProcessingError(message, { code, cause });
  }
  async function calculateJwkThumbprint(jwk) {
    let components;
    switch (jwk.kty) {
      case "EC":
        components = {
          crv: jwk.crv,
          kty: jwk.kty,
          x: jwk.x,
          y: jwk.y
        };
        break;
      case "OKP":
        components = {
          crv: jwk.crv,
          kty: jwk.kty,
          x: jwk.x
        };
        break;
      case "AKP":
        components = {
          alg: jwk.alg,
          kty: jwk.kty,
          pub: jwk.pub
        };
        break;
      case "RSA":
        components = {
          e: jwk.e,
          kty: jwk.kty,
          n: jwk.n
        };
        break;
      default:
        throw new UnsupportedOperationError("unsupported JWK key type", { cause: jwk });
    }
    return b64u(await crypto.subtle.digest("SHA-256", buf(JSON.stringify(components))));
  }
  function assertCryptoKey(key, it) {
    if (!(key instanceof CryptoKey)) {
      throw CodedTypeError(`${it} must be a CryptoKey`, ERR_INVALID_ARG_TYPE);
    }
  }
  function assertPrivateKey(key, it) {
    assertCryptoKey(key, it);
    if (key.type !== "private") {
      throw CodedTypeError(`${it} must be a private CryptoKey`, ERR_INVALID_ARG_VALUE);
    }
  }
  function assertPublicKey(key, it) {
    assertCryptoKey(key, it);
    if (key.type !== "public") {
      throw CodedTypeError(`${it} must be a public CryptoKey`, ERR_INVALID_ARG_VALUE);
    }
  }
  function isJsonObject(input) {
    if (input === null || typeof input !== "object" || Array.isArray(input)) {
      return false;
    }
    return true;
  }
  function prepareHeaders(input) {
    if (looseInstanceOf(input, Headers)) {
      input = Object.fromEntries(input.entries());
    }
    const headers2 = new Headers(input ?? {});
    if (USER_AGENT && !headers2.has("user-agent")) {
      headers2.set("user-agent", USER_AGENT);
    }
    if (headers2.has("authorization")) {
      throw CodedTypeError('"options.headers" must not include the "authorization" header name', ERR_INVALID_ARG_VALUE);
    }
    return headers2;
  }
  function signal(url, value) {
    if (value !== void 0) {
      if (typeof value === "function") {
        value = value(url.href);
      }
      if (!(value instanceof AbortSignal)) {
        throw CodedTypeError('"options.signal" must return or be an instance of AbortSignal', ERR_INVALID_ARG_TYPE);
      }
      return value;
    }
    return void 0;
  }
  function replaceDoubleSlash(pathname) {
    if (pathname.includes("//")) {
      return pathname.replace("//", "/");
    }
    return pathname;
  }
  function prependWellKnown(url, wellKnown, allowTerminatingSlash = false) {
    if (url.pathname === "/") {
      url.pathname = wellKnown;
    } else {
      url.pathname = replaceDoubleSlash(`${wellKnown}/${allowTerminatingSlash ? url.pathname : url.pathname.replace(/(\/)$/, "")}`);
    }
    return url;
  }
  function appendWellKnown(url, wellKnown) {
    url.pathname = replaceDoubleSlash(`${url.pathname}/${wellKnown}`);
    return url;
  }
  async function performDiscovery(input, urlName, transform, options) {
    if (!(input instanceof URL)) {
      throw CodedTypeError(`"${urlName}" must be an instance of URL`, ERR_INVALID_ARG_TYPE);
    }
    checkProtocol(input, options?.[allowInsecureRequests] !== true);
    const url = transform(new URL(input.href));
    const headers2 = prepareHeaders(options?.headers);
    headers2.set("accept", "application/json");
    return (options?.[customFetch] || fetch)(url.href, {
      body: void 0,
      headers: Object.fromEntries(headers2.entries()),
      method: "GET",
      redirect: "manual",
      signal: signal(url, options?.signal)
    });
  }
  async function discoveryRequest(issuerIdentifier, options) {
    return performDiscovery(issuerIdentifier, "issuerIdentifier", (url) => {
      switch (options?.algorithm) {
        case void 0:
        case "oidc":
          appendWellKnown(url, ".well-known/openid-configuration");
          break;
        case "oauth2":
          prependWellKnown(url, ".well-known/oauth-authorization-server");
          break;
        default:
          throw CodedTypeError('"options.algorithm" must be "oidc" (default), or "oauth2"', ERR_INVALID_ARG_VALUE);
      }
      return url;
    }, options);
  }
  function assertNumber(input, allow0, it, code, cause) {
    try {
      if (typeof input !== "number" || !Number.isFinite(input)) {
        throw CodedTypeError(`${it} must be a number`, ERR_INVALID_ARG_TYPE, cause);
      }
      if (input > 0)
        return;
      if (allow0) {
        if (input !== 0) {
          throw CodedTypeError(`${it} must be a non-negative number`, ERR_INVALID_ARG_VALUE, cause);
        }
        return;
      }
      throw CodedTypeError(`${it} must be a positive number`, ERR_INVALID_ARG_VALUE, cause);
    } catch (err) {
      if (code) {
        throw OPE(err.message, code, cause);
      }
      throw err;
    }
  }
  function assertString(input, it, code, cause) {
    try {
      if (typeof input !== "string") {
        throw CodedTypeError(`${it} must be a string`, ERR_INVALID_ARG_TYPE, cause);
      }
      if (input.length === 0) {
        throw CodedTypeError(`${it} must not be empty`, ERR_INVALID_ARG_VALUE, cause);
      }
    } catch (err) {
      if (code) {
        throw OPE(err.message, code, cause);
      }
      throw err;
    }
  }
  async function processDiscoveryResponse(expectedIssuerIdentifier, response) {
    const expected = expectedIssuerIdentifier;
    if (!(expected instanceof URL) && expected !== _nodiscoverycheck) {
      throw CodedTypeError('"expectedIssuerIdentifier" must be an instance of URL', ERR_INVALID_ARG_TYPE);
    }
    if (!looseInstanceOf(response, Response)) {
      throw CodedTypeError('"response" must be an instance of Response', ERR_INVALID_ARG_TYPE);
    }
    if (response.status !== 200) {
      throw OPE('"response" is not a conform Authorization Server Metadata response (unexpected HTTP status code)', RESPONSE_IS_NOT_CONFORM, response);
    }
    assertReadableResponse(response);
    const json = await getResponseJsonBody(response);
    assertString(json.issuer, '"response" body "issuer" property', INVALID_RESPONSE, { body: json });
    if (expected !== _nodiscoverycheck && new URL(json.issuer).href !== expected.href) {
      throw OPE('"response" body "issuer" property does not match the expected value', JSON_ATTRIBUTE_COMPARISON, { expected: expected.href, body: json, attribute: "issuer" });
    }
    return json;
  }
  function assertApplicationJson(response) {
    assertContentType(response, "application/json");
  }
  function notJson(response, ...types) {
    let msg = '"response" content-type must be ';
    if (types.length > 2) {
      const last = types.pop();
      msg += `${types.join(", ")}, or ${last}`;
    } else if (types.length === 2) {
      msg += `${types[0]} or ${types[1]}`;
    } else {
      msg += types[0];
    }
    return OPE(msg, RESPONSE_IS_NOT_JSON, response);
  }
  function assertContentType(response, contentType) {
    if (getContentType(response) !== contentType) {
      throw notJson(response, contentType);
    }
  }
  function randomBytes() {
    return b64u(crypto.getRandomValues(new Uint8Array(32)));
  }
  function psAlg(key) {
    switch (key.algorithm.hash.name) {
      case "SHA-256":
        return "PS256";
      case "SHA-384":
        return "PS384";
      case "SHA-512":
        return "PS512";
      default:
        throw new UnsupportedOperationError("unsupported RsaHashedKeyAlgorithm hash name", {
          cause: key
        });
    }
  }
  function rsAlg(key) {
    switch (key.algorithm.hash.name) {
      case "SHA-256":
        return "RS256";
      case "SHA-384":
        return "RS384";
      case "SHA-512":
        return "RS512";
      default:
        throw new UnsupportedOperationError("unsupported RsaHashedKeyAlgorithm hash name", {
          cause: key
        });
    }
  }
  function esAlg(key) {
    switch (key.algorithm.namedCurve) {
      case "P-256":
        return "ES256";
      case "P-384":
        return "ES384";
      case "P-521":
        return "ES512";
      default:
        throw new UnsupportedOperationError("unsupported EcKeyAlgorithm namedCurve", { cause: key });
    }
  }
  function keyToJws(key) {
    switch (key.algorithm.name) {
      case "RSA-PSS":
        return psAlg(key);
      case "RSASSA-PKCS1-v1_5":
        return rsAlg(key);
      case "ECDSA":
        return esAlg(key);
      case "Ed25519":
      case "ML-DSA-44":
      case "ML-DSA-65":
      case "ML-DSA-87":
        return key.algorithm.name;
      case "EdDSA":
        return "Ed25519";
      default:
        throw new UnsupportedOperationError("unsupported CryptoKey algorithm name", { cause: key });
    }
  }
  function getClockSkew(client) {
    const skew = client?.[clockSkew];
    return typeof skew === "number" && Number.isFinite(skew) ? skew : 0;
  }
  function getClockTolerance(client) {
    const tolerance = client?.[clockTolerance];
    return typeof tolerance === "number" && Number.isFinite(tolerance) && Math.sign(tolerance) !== -1 ? tolerance : 30;
  }
  function epochTime() {
    return Math.floor(Date.now() / 1e3);
  }
  function assertAs(as) {
    if (typeof as !== "object" || as === null) {
      throw CodedTypeError('"as" must be an object', ERR_INVALID_ARG_TYPE);
    }
    assertString(as.issuer, '"as.issuer"');
  }
  function assertClient(client) {
    if (typeof client !== "object" || client === null) {
      throw CodedTypeError('"client" must be an object', ERR_INVALID_ARG_TYPE);
    }
    assertString(client.client_id, '"client.client_id"');
  }
  function ClientSecretPost(clientSecret) {
    assertString(clientSecret, '"clientSecret"');
    return (_as, client, body, _headers) => {
      body.set("client_id", client.client_id);
      body.set("client_secret", clientSecret);
    };
  }
  function None() {
    return (_as, client, body, _headers) => {
      body.set("client_id", client.client_id);
    };
  }
  async function signJwt(header, payload, key) {
    if (!key.usages.includes("sign")) {
      throw CodedTypeError('CryptoKey instances used for signing assertions must include "sign" in their "usages"', ERR_INVALID_ARG_VALUE);
    }
    const input = `${b64u(buf(JSON.stringify(header)))}.${b64u(buf(JSON.stringify(payload)))}`;
    const signature = b64u(await crypto.subtle.sign(keyToSubtle(key), key, buf(input)));
    return `${input}.${signature}`;
  }
  var jwkCache;
  async function getSetPublicJwkCache(key, alg) {
    const { kty, e: e2, n, x, y, crv, pub } = await crypto.subtle.exportKey("jwk", key);
    const jwk = { kty, e: e2, n, x, y, crv, pub };
    if (kty === "AKP")
      jwk.alg = alg;
    jwkCache.set(key, jwk);
    return jwk;
  }
  async function publicJwk(key, alg) {
    jwkCache ||= /* @__PURE__ */ new WeakMap();
    return jwkCache.get(key) || getSetPublicJwkCache(key, alg);
  }
  var URLParse = URL.parse ? (url, base) => URL.parse(url, base) : (url, base) => {
    try {
      return new URL(url, base);
    } catch {
      return null;
    }
  };
  function checkProtocol(url, enforceHttps) {
    if (enforceHttps && url.protocol !== "https:") {
      throw OPE("only requests to HTTPS are allowed", HTTP_REQUEST_FORBIDDEN, url);
    }
    if (url.protocol !== "https:" && url.protocol !== "http:") {
      throw OPE("only HTTP and HTTPS requests are allowed", REQUEST_PROTOCOL_FORBIDDEN, url);
    }
  }
  function validateEndpoint(value, endpoint, useMtlsAlias, enforceHttps) {
    let url;
    if (typeof value !== "string" || !(url = URLParse(value))) {
      throw OPE(`authorization server metadata does not contain a valid ${useMtlsAlias ? `"as.mtls_endpoint_aliases.${endpoint}"` : `"as.${endpoint}"`}`, value === void 0 ? MISSING_SERVER_METADATA : INVALID_SERVER_METADATA, { attribute: useMtlsAlias ? `mtls_endpoint_aliases.${endpoint}` : endpoint });
    }
    checkProtocol(url, enforceHttps);
    return url;
  }
  function resolveEndpoint(as, endpoint, useMtlsAlias, enforceHttps) {
    if (useMtlsAlias && as.mtls_endpoint_aliases && endpoint in as.mtls_endpoint_aliases) {
      return validateEndpoint(as.mtls_endpoint_aliases[endpoint], endpoint, useMtlsAlias, enforceHttps);
    }
    return validateEndpoint(as[endpoint], endpoint, useMtlsAlias, enforceHttps);
  }
  var DPoPHandler = class {
    #header;
    #privateKey;
    #publicKey;
    #clockSkew;
    #modifyAssertion;
    #map;
    #jkt;
    constructor(client, keyPair, options) {
      assertPrivateKey(keyPair?.privateKey, '"DPoP.privateKey"');
      assertPublicKey(keyPair?.publicKey, '"DPoP.publicKey"');
      if (!keyPair.publicKey.extractable) {
        throw CodedTypeError('"DPoP.publicKey.extractable" must be true', ERR_INVALID_ARG_VALUE);
      }
      this.#modifyAssertion = options?.[modifyAssertion];
      this.#clockSkew = getClockSkew(client);
      this.#privateKey = keyPair.privateKey;
      this.#publicKey = keyPair.publicKey;
      branded.add(this);
    }
    #get(key) {
      this.#map ||= /* @__PURE__ */ new Map();
      let item = this.#map.get(key);
      if (item) {
        this.#map.delete(key);
        this.#map.set(key, item);
      }
      return item;
    }
    #set(key, val) {
      this.#map ||= /* @__PURE__ */ new Map();
      this.#map.delete(key);
      if (this.#map.size === 100) {
        this.#map.delete(this.#map.keys().next().value);
      }
      this.#map.set(key, val);
    }
    async calculateThumbprint() {
      if (!this.#jkt) {
        const jwk = await crypto.subtle.exportKey("jwk", this.#publicKey);
        this.#jkt ||= await calculateJwkThumbprint(jwk);
      }
      return this.#jkt;
    }
    async addProof(url, headers2, htm, accessToken) {
      const alg = keyToJws(this.#privateKey);
      this.#header ||= {
        alg,
        typ: "dpop+jwt",
        jwk: await publicJwk(this.#publicKey, alg)
      };
      const nonce = this.#get(url.origin);
      const now = epochTime() + this.#clockSkew;
      const payload = {
        iat: now,
        jti: randomBytes(),
        htm,
        nonce,
        htu: `${url.origin}${url.pathname}`,
        ath: accessToken ? b64u(await crypto.subtle.digest("SHA-256", buf(accessToken))) : void 0
      };
      this.#modifyAssertion?.(this.#header, payload);
      headers2.set("dpop", await signJwt(this.#header, payload, this.#privateKey));
    }
    cacheNonce(response, url) {
      try {
        const nonce = response.headers.get("dpop-nonce");
        if (nonce) {
          this.#set(url.origin, nonce);
        }
      } catch {
      }
    }
  };
  function isDPoPNonceError(err) {
    if (err instanceof WWWAuthenticateChallengeError) {
      return err.cause.some((challenge) => challenge.scheme === "dpop" && challenge.parameters.error === "use_dpop_nonce");
    }
    if (err instanceof ResponseBodyError) {
      return err.error === "use_dpop_nonce";
    }
    return false;
  }
  function DPoP(client, keyPair, options) {
    return new DPoPHandler(client, keyPair, options);
  }
  var ResponseBodyError = class extends Error {
    cause;
    code;
    error;
    status;
    error_description;
    response;
    constructor(message, options) {
      super(message, options);
      this.name = this.constructor.name;
      this.code = RESPONSE_BODY_ERROR;
      this.cause = options.cause;
      this.error = options.cause.error;
      this.status = options.response.status;
      this.error_description = options.cause.error_description;
      Object.defineProperty(this, "response", { enumerable: false, value: options.response });
      Error.captureStackTrace?.(this, this.constructor);
    }
  };
  var AuthorizationResponseError = class extends Error {
    cause;
    code;
    error;
    error_description;
    constructor(message, options) {
      super(message, options);
      this.name = this.constructor.name;
      this.code = AUTHORIZATION_RESPONSE_ERROR;
      this.cause = options.cause;
      this.error = options.cause.get("error");
      this.error_description = options.cause.get("error_description") ?? void 0;
      Error.captureStackTrace?.(this, this.constructor);
    }
  };
  var WWWAuthenticateChallengeError = class extends Error {
    cause;
    code;
    response;
    status;
    constructor(message, options) {
      super(message, options);
      this.name = this.constructor.name;
      this.code = WWW_AUTHENTICATE_CHALLENGE;
      this.cause = options.cause;
      this.status = options.response.status;
      this.response = options.response;
      Object.defineProperty(this, "response", { enumerable: false });
      Error.captureStackTrace?.(this, this.constructor);
    }
  };
  var tokenMatch = "[a-zA-Z0-9!#$%&\\'\\*\\+\\-\\.\\^_`\\|~]+";
  var token68Match = "[a-zA-Z0-9\\-\\._\\~\\+\\/]+={0,2}";
  var quotedMatch = '"((?:[^"\\\\]|\\\\[\\s\\S])*)"';
  var quotedParamMatcher = "(" + tokenMatch + ")\\s*=\\s*" + quotedMatch;
  var paramMatcher = "(" + tokenMatch + ")\\s*=\\s*(" + tokenMatch + ")";
  var schemeRE = new RegExp("^[,\\s]*(" + tokenMatch + ")");
  var quotedParamRE = new RegExp("^[,\\s]*" + quotedParamMatcher + "[,\\s]*(.*)");
  var unquotedParamRE = new RegExp("^[,\\s]*" + paramMatcher + "[,\\s]*(.*)");
  var token68ParamRE = new RegExp("^(" + token68Match + ")(?:$|[,\\s])(.*)");
  function parseWwwAuthenticateChallenges(response) {
    if (!looseInstanceOf(response, Response)) {
      throw CodedTypeError('"response" must be an instance of Response', ERR_INVALID_ARG_TYPE);
    }
    const header = response.headers.get("www-authenticate");
    if (header === null) {
      return void 0;
    }
    const challenges = [];
    let rest = header;
    while (rest) {
      let match = rest.match(schemeRE);
      const scheme = match?.["1"].toLowerCase();
      if (!scheme) {
        return void 0;
      }
      const afterScheme = rest.substring(match[0].length);
      if (afterScheme && !afterScheme.match(/^[\s,]/)) {
        return void 0;
      }
      const spaceMatch = afterScheme.match(/^\s+(.*)$/);
      const hasParameters = !!spaceMatch;
      rest = spaceMatch ? spaceMatch[1] : void 0;
      const parameters = {};
      let token68;
      if (hasParameters) {
        while (rest) {
          let key;
          let value;
          if (match = rest.match(quotedParamRE)) {
            ;
            [, key, value, rest] = match;
            if (value.includes("\\")) {
              value = value.replace(/\\([\s\S])/g, "$1");
            }
            parameters[key.toLowerCase()] = value;
            continue;
          }
          if (match = rest.match(unquotedParamRE)) {
            ;
            [, key, value, rest] = match;
            parameters[key.toLowerCase()] = value;
            continue;
          }
          if (match = rest.match(token68ParamRE)) {
            if (Object.keys(parameters).length) {
              break;
            }
            ;
            [, token68, rest] = match;
            break;
          }
          return void 0;
        }
      } else {
        rest = afterScheme || void 0;
      }
      const challenge = { scheme, parameters };
      if (token68) {
        challenge.token68 = token68;
      }
      challenges.push(challenge);
    }
    if (!challenges.length) {
      return void 0;
    }
    return challenges;
  }
  async function parseOAuthResponseErrorBody(response) {
    if (response.status > 399 && response.status < 500) {
      assertReadableResponse(response);
      assertApplicationJson(response);
      try {
        const json = await response.clone().json();
        if (isJsonObject(json) && typeof json.error === "string" && json.error.length) {
          return json;
        }
      } catch {
      }
    }
    return void 0;
  }
  async function checkOAuthBodyError(response, expected, label) {
    if (response.status !== expected) {
      checkAuthenticationChallenges(response);
      let err;
      if (err = await parseOAuthResponseErrorBody(response)) {
        await response.body?.cancel();
        throw new ResponseBodyError("server responded with an error in the response body", {
          cause: err,
          response
        });
      }
      throw OPE(`"response" is not a conform ${label} response (unexpected HTTP status code)`, RESPONSE_IS_NOT_CONFORM, response);
    }
  }
  function assertDPoP(option) {
    if (!branded.has(option)) {
      throw CodedTypeError('"options.DPoP" is not a valid DPoPHandle', ERR_INVALID_ARG_VALUE);
    }
  }
  var skipSubjectCheck = Symbol();
  function getContentType(input) {
    return input.headers.get("content-type")?.split(";")[0];
  }
  async function authenticatedRequest(as, client, clientAuthentication, url, body, headers2, options) {
    await clientAuthentication(as, client, body, headers2);
    headers2.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8");
    return (options?.[customFetch] || fetch)(url.href, {
      body,
      headers: Object.fromEntries(headers2.entries()),
      method: "POST",
      redirect: "manual",
      signal: signal(url, options?.signal)
    });
  }
  async function tokenEndpointRequest(as, client, clientAuthentication, grantType, parameters, options) {
    const url = resolveEndpoint(as, "token_endpoint", client.use_mtls_endpoint_aliases, options?.[allowInsecureRequests] !== true);
    parameters.set("grant_type", grantType);
    const headers2 = prepareHeaders(options?.headers);
    headers2.set("accept", "application/json");
    if (options?.DPoP !== void 0) {
      assertDPoP(options.DPoP);
      await options.DPoP.addProof(url, headers2, "POST");
    }
    const response = await authenticatedRequest(as, client, clientAuthentication, url, parameters, headers2, options);
    options?.DPoP?.cacheNonce(response, url);
    return response;
  }
  async function refreshTokenGrantRequest(as, client, clientAuthentication, refreshToken, options) {
    assertAs(as);
    assertClient(client);
    assertString(refreshToken, '"refreshToken"');
    const parameters = new URLSearchParams(options?.additionalParameters);
    parameters.set("refresh_token", refreshToken);
    return tokenEndpointRequest(as, client, clientAuthentication, "refresh_token", parameters, options);
  }
  var idTokenClaims = /* @__PURE__ */ new WeakMap();
  var jwtRefs = /* @__PURE__ */ new WeakMap();
  function getValidatedIdTokenClaims(ref) {
    if (!ref.id_token) {
      return void 0;
    }
    const claims = idTokenClaims.get(ref);
    if (!claims) {
      throw CodedTypeError('"ref" was already garbage collected or did not resolve from the proper sources', ERR_INVALID_ARG_VALUE);
    }
    return claims;
  }
  async function processGenericAccessTokenResponse(as, client, response, additionalRequiredIdTokenClaims, decryptFn, recognizedTokenTypes) {
    assertAs(as);
    assertClient(client);
    if (!looseInstanceOf(response, Response)) {
      throw CodedTypeError('"response" must be an instance of Response', ERR_INVALID_ARG_TYPE);
    }
    await checkOAuthBodyError(response, 200, "Token Endpoint");
    assertReadableResponse(response);
    const json = await getResponseJsonBody(response);
    assertString(json.access_token, '"response" body "access_token" property', INVALID_RESPONSE, {
      body: json
    });
    assertString(json.token_type, '"response" body "token_type" property', INVALID_RESPONSE, {
      body: json
    });
    json.token_type = json.token_type.toLowerCase();
    if (json.expires_in !== void 0) {
      let expiresIn = typeof json.expires_in !== "number" ? parseFloat(json.expires_in) : json.expires_in;
      assertNumber(expiresIn, true, '"response" body "expires_in" property', INVALID_RESPONSE, {
        body: json
      });
      json.expires_in = expiresIn;
    }
    if (json.refresh_token !== void 0) {
      assertString(json.refresh_token, '"response" body "refresh_token" property', INVALID_RESPONSE, {
        body: json
      });
    }
    if (json.scope !== void 0 && typeof json.scope !== "string") {
      throw OPE('"response" body "scope" property must be a string', INVALID_RESPONSE, { body: json });
    }
    if (json.id_token !== void 0) {
      assertString(json.id_token, '"response" body "id_token" property', INVALID_RESPONSE, {
        body: json
      });
      const requiredClaims = [];
      if (client.require_auth_time === true) {
        requiredClaims.push("auth_time");
      }
      if (client.default_max_age !== void 0) {
        assertNumber(client.default_max_age, true, '"client.default_max_age"');
        requiredClaims.push("auth_time");
      }
      if (additionalRequiredIdTokenClaims?.length) {
        requiredClaims.push(...additionalRequiredIdTokenClaims);
      }
      const { claims, jwt } = await validateIdTokenClaims(as, client, json.id_token, requiredClaims, decryptFn);
      validateIdTokenAuthorizedParty(client, claims);
      validateIdTokenAuthTimeClaim(claims);
      jwtRefs.set(response, jwt);
      idTokenClaims.set(json, claims);
    }
    if (recognizedTokenTypes?.[json.token_type] !== void 0) {
      recognizedTokenTypes[json.token_type](response, json);
    } else if (json.token_type !== "dpop" && json.token_type !== "bearer") {
      throw new UnsupportedOperationError("unsupported `token_type` value", { cause: { body: json } });
    }
    return json;
  }
  function checkAuthenticationChallenges(response) {
    let challenges;
    if (challenges = parseWwwAuthenticateChallenges(response)) {
      throw new WWWAuthenticateChallengeError("server responded with a challenge in the WWW-Authenticate HTTP Header", { cause: challenges, response });
    }
  }
  async function processRefreshTokenResponse(as, client, response, options) {
    return processGenericAccessTokenResponse(as, client, response, void 0, options?.[jweDecrypt], options?.recognizedTokenTypes);
  }
  function validateAudience(expected, result) {
    if (Array.isArray(result.claims.aud)) {
      if (!result.claims.aud.includes(expected)) {
        throw OPE('unexpected JWT "aud" (audience) claim value', JWT_CLAIM_COMPARISON, {
          expected,
          claims: result.claims,
          claim: "aud"
        });
      }
    } else if (result.claims.aud !== expected) {
      throw OPE('unexpected JWT "aud" (audience) claim value', JWT_CLAIM_COMPARISON, {
        expected,
        claims: result.claims,
        claim: "aud"
      });
    }
    return result;
  }
  function validateIssuer(as, result) {
    const expected = as[_expectedIssuer]?.(result) ?? as.issuer;
    if (result.claims.iss !== expected) {
      throw OPE('unexpected JWT "iss" (issuer) claim value', JWT_CLAIM_COMPARISON, {
        expected,
        claims: result.claims,
        claim: "iss"
      });
    }
    return result;
  }
  var branded = /* @__PURE__ */ new WeakSet();
  var nopkce = Symbol();
  var jwtClaimNames = {
    aud: "audience",
    c_hash: "code hash",
    client_id: "client id",
    exp: "expiration time",
    iat: "issued at",
    iss: "issuer",
    jti: "jwt id",
    nonce: "nonce",
    s_hash: "state hash",
    sub: "subject",
    ath: "access token hash",
    htm: "http method",
    htu: "http uri",
    cnf: "confirmation",
    auth_time: "authentication time"
  };
  function validatePresence(required, result) {
    for (const claim of required) {
      if (result.claims[claim] === void 0) {
        throw OPE(`JWT "${claim}" (${jwtClaimNames[claim]}) claim missing`, INVALID_RESPONSE, {
          claims: result.claims
        });
      }
    }
    return result;
  }
  function validateStringClaim(claim, result) {
    if (typeof result.claims[claim] !== "string") {
      throw OPE(`unexpected JWT "${claim}" (${jwtClaimNames[claim]}) claim type`, INVALID_RESPONSE, {
        claims: result.claims
      });
    }
    return result;
  }
  function validateIdTokenClaims(as, client, idToken, requiredClaims, decryptFn) {
    return validateJwt(idToken, checkSigningAlgorithm.bind(void 0, client.id_token_signed_response_alg, as.id_token_signing_alg_values_supported, "RS256"), getClockSkew(client), getClockTolerance(client), decryptFn).then(validatePresence.bind(void 0, ["aud", "exp", "iat", "iss", "sub", ...requiredClaims])).then(validateIssuer.bind(void 0, as)).then(validateAudience.bind(void 0, client.client_id)).then(validateStringClaim.bind(void 0, "sub"));
  }
  function validateIdTokenAuthTimeClaim(claims) {
    if (claims.auth_time !== void 0) {
      assertNumber(claims.auth_time, true, 'ID Token "auth_time" (authentication time)', INVALID_RESPONSE, { claims });
    }
  }
  function validateIdTokenAuthorizedParty(client, claims) {
    if (Array.isArray(claims.aud) && claims.aud.length !== 1) {
      if (claims.azp === void 0) {
        throw OPE('ID Token "aud" (audience) claim includes additional untrusted audiences', JWT_CLAIM_COMPARISON, { claims, claim: "aud" });
      }
      if (claims.azp !== client.client_id) {
        throw OPE('unexpected ID Token "azp" (authorized party) claim value', JWT_CLAIM_COMPARISON, {
          expected: client.client_id,
          claims,
          claim: "azp"
        });
      }
    }
  }
  var expectNoNonce = Symbol();
  var skipAuthTimeCheck = Symbol();
  var WWW_AUTHENTICATE_CHALLENGE = "OAUTH_WWW_AUTHENTICATE_CHALLENGE";
  var RESPONSE_BODY_ERROR = "OAUTH_RESPONSE_BODY_ERROR";
  var UNSUPPORTED_OPERATION = "OAUTH_UNSUPPORTED_OPERATION";
  var AUTHORIZATION_RESPONSE_ERROR = "OAUTH_AUTHORIZATION_RESPONSE_ERROR";
  var PARSE_ERROR = "OAUTH_PARSE_ERROR";
  var INVALID_RESPONSE = "OAUTH_INVALID_RESPONSE";
  var RESPONSE_IS_NOT_JSON = "OAUTH_RESPONSE_IS_NOT_JSON";
  var RESPONSE_IS_NOT_CONFORM = "OAUTH_RESPONSE_IS_NOT_CONFORM";
  var HTTP_REQUEST_FORBIDDEN = "OAUTH_HTTP_REQUEST_FORBIDDEN";
  var REQUEST_PROTOCOL_FORBIDDEN = "OAUTH_REQUEST_PROTOCOL_FORBIDDEN";
  var JWT_TIMESTAMP_CHECK = "OAUTH_JWT_TIMESTAMP_CHECK_FAILED";
  var JWT_CLAIM_COMPARISON = "OAUTH_JWT_CLAIM_COMPARISON_FAILED";
  var JSON_ATTRIBUTE_COMPARISON = "OAUTH_JSON_ATTRIBUTE_COMPARISON_FAILED";
  var MISSING_SERVER_METADATA = "OAUTH_MISSING_SERVER_METADATA";
  var INVALID_SERVER_METADATA = "OAUTH_INVALID_SERVER_METADATA";
  function assertReadableResponse(response) {
    if (response.bodyUsed) {
      throw CodedTypeError('"response" body has been used already', ERR_INVALID_ARG_VALUE);
    }
  }
  function checkRsaKeyAlgorithm(key) {
    const { algorithm } = key;
    if (typeof algorithm.modulusLength !== "number" || algorithm.modulusLength < 2048) {
      throw new UnsupportedOperationError(`unsupported ${algorithm.name} modulusLength`, {
        cause: key
      });
    }
  }
  function ecdsaHashName(key) {
    const { algorithm } = key;
    switch (algorithm.namedCurve) {
      case "P-256":
        return "SHA-256";
      case "P-384":
        return "SHA-384";
      case "P-521":
        return "SHA-512";
      default:
        throw new UnsupportedOperationError("unsupported ECDSA namedCurve", { cause: key });
    }
  }
  function keyToSubtle(key) {
    switch (key.algorithm.name) {
      case "ECDSA":
        return {
          name: key.algorithm.name,
          hash: ecdsaHashName(key)
        };
      case "RSA-PSS": {
        checkRsaKeyAlgorithm(key);
        switch (key.algorithm.hash.name) {
          case "SHA-256":
          case "SHA-384":
          case "SHA-512":
            return {
              name: key.algorithm.name,
              saltLength: parseInt(key.algorithm.hash.name.slice(-3), 10) >> 3
            };
          default:
            throw new UnsupportedOperationError("unsupported RSA-PSS hash name", { cause: key });
        }
      }
      case "RSASSA-PKCS1-v1_5":
        checkRsaKeyAlgorithm(key);
        return key.algorithm.name;
      case "ML-DSA-44":
      case "ML-DSA-65":
      case "ML-DSA-87":
      case "Ed25519":
        return key.algorithm.name;
    }
    throw new UnsupportedOperationError("unsupported CryptoKey algorithm name", { cause: key });
  }
  async function validateJwt(jws, checkAlg, clockSkew2, clockTolerance2, decryptJwt) {
    let { 0: protectedHeader, 1: payload, length } = jws.split(".");
    if (length === 5) {
      if (decryptJwt !== void 0) {
        jws = await decryptJwt(jws);
        ({ 0: protectedHeader, 1: payload, length } = jws.split("."));
      } else {
        throw new UnsupportedOperationError("JWE decryption is not configured", { cause: jws });
      }
    }
    if (length !== 3) {
      throw OPE("Invalid JWT", INVALID_RESPONSE, jws);
    }
    let header;
    try {
      header = JSON.parse(buf(b64u(protectedHeader)));
    } catch (cause) {
      throw OPE("failed to parse JWT Header body as base64url encoded JSON", PARSE_ERROR, cause);
    }
    if (!isJsonObject(header)) {
      throw OPE("JWT Header must be a top level object", INVALID_RESPONSE, jws);
    }
    checkAlg(header);
    if (header.crit !== void 0) {
      throw new UnsupportedOperationError('no JWT "crit" header parameter extensions are supported', {
        cause: { header }
      });
    }
    let claims;
    try {
      claims = JSON.parse(buf(b64u(payload)));
    } catch (cause) {
      throw OPE("failed to parse JWT Payload body as base64url encoded JSON", PARSE_ERROR, cause);
    }
    if (!isJsonObject(claims)) {
      throw OPE("JWT Payload must be a top level object", INVALID_RESPONSE, jws);
    }
    const now = epochTime() + clockSkew2;
    if (claims.exp !== void 0) {
      if (typeof claims.exp !== "number") {
        throw OPE('unexpected JWT "exp" (expiration time) claim type', INVALID_RESPONSE, { claims });
      }
      if (claims.exp <= now - clockTolerance2) {
        throw OPE('unexpected JWT "exp" (expiration time) claim value, expiration is past current timestamp', JWT_TIMESTAMP_CHECK, { claims, now, tolerance: clockTolerance2, claim: "exp" });
      }
    }
    if (claims.iat !== void 0) {
      if (typeof claims.iat !== "number") {
        throw OPE('unexpected JWT "iat" (issued at) claim type', INVALID_RESPONSE, { claims });
      }
    }
    if (claims.iss !== void 0) {
      if (typeof claims.iss !== "string") {
        throw OPE('unexpected JWT "iss" (issuer) claim type', INVALID_RESPONSE, { claims });
      }
    }
    if (claims.nbf !== void 0) {
      if (typeof claims.nbf !== "number") {
        throw OPE('unexpected JWT "nbf" (not before) claim type', INVALID_RESPONSE, { claims });
      }
      if (claims.nbf > now + clockTolerance2) {
        throw OPE('unexpected JWT "nbf" (not before) claim value', JWT_TIMESTAMP_CHECK, {
          claims,
          now,
          tolerance: clockTolerance2,
          claim: "nbf"
        });
      }
    }
    if (claims.aud !== void 0) {
      if (typeof claims.aud !== "string" && !Array.isArray(claims.aud)) {
        throw OPE('unexpected JWT "aud" (audience) claim type', INVALID_RESPONSE, { claims });
      }
    }
    return { header, claims, jwt: jws };
  }
  function checkSigningAlgorithm(client, issuer, fallback, header) {
    if (client !== void 0) {
      if (typeof client === "string" ? header.alg !== client : !client.includes(header.alg)) {
        throw OPE('unexpected JWT "alg" header parameter', INVALID_RESPONSE, {
          header,
          expected: client,
          reason: "client configuration"
        });
      }
      return;
    }
    if (Array.isArray(issuer)) {
      if (!issuer.includes(header.alg)) {
        throw OPE('unexpected JWT "alg" header parameter', INVALID_RESPONSE, {
          header,
          expected: issuer,
          reason: "authorization server metadata"
        });
      }
      return;
    }
    if (fallback !== void 0) {
      if (typeof fallback === "string" ? header.alg !== fallback : typeof fallback === "function" ? !fallback(header.alg) : !fallback.includes(header.alg)) {
        throw OPE('unexpected JWT "alg" header parameter', INVALID_RESPONSE, {
          header,
          expected: fallback,
          reason: "default value"
        });
      }
      return;
    }
    throw OPE('missing client or server configuration to verify used JWT "alg" header parameter', void 0, { client, issuer, fallback });
  }
  var skipStateCheck = Symbol();
  var expectNoState = Symbol();
  async function getResponseJsonBody(response, check2 = assertApplicationJson) {
    let json;
    try {
      json = await response.json();
    } catch (cause) {
      check2(response);
      throw OPE('failed to parse "response" body as JSON', PARSE_ERROR, cause);
    }
    if (!isJsonObject(json)) {
      throw OPE('"response" body must be a top level object', INVALID_RESPONSE, { body: json });
    }
    return json;
  }
  var _nodiscoverycheck = Symbol();
  var _expectedIssuer = Symbol();

  // node_modules/openid-client/build/index.js
  var headers;
  var USER_AGENT2;
  if (typeof navigator === "undefined" || !navigator.userAgent?.startsWith?.("Mozilla/5.0 ")) {
    const NAME = "openid-client";
    const VERSION = "v6.8.8";
    USER_AGENT2 = `${NAME}/${VERSION}`;
    headers = { "user-agent": USER_AGENT2 };
  }
  var int = (config) => {
    return props.get(config);
  };
  var props;
  function ClientSecretPost2(clientSecret) {
    if (clientSecret !== void 0) {
      return ClientSecretPost(clientSecret);
    }
    const cache = /* @__PURE__ */ new WeakMap();
    return (as, client, body, headers2) => {
      let auth;
      if (!(auth = cache.get(client))) {
        assertString2(client.client_secret, '"metadata.client_secret"');
        auth = ClientSecretPost(client.client_secret);
        cache.set(client, auth);
      }
      return auth(as, client, body, headers2);
    };
  }
  function assertString2(input, it) {
    if (typeof input !== "string") {
      throw CodedTypeError2(`${it} must be a string`, ERR_INVALID_ARG_TYPE2);
    }
    if (input.length === 0) {
      throw CodedTypeError2(`${it} must not be empty`, ERR_INVALID_ARG_VALUE2);
    }
  }
  function None2() {
    return None();
  }
  var customFetch2 = customFetch;
  var ERR_INVALID_ARG_VALUE2 = "ERR_INVALID_ARG_VALUE";
  var ERR_INVALID_ARG_TYPE2 = "ERR_INVALID_ARG_TYPE";
  function CodedTypeError2(message, code, cause) {
    const err = new TypeError(message, { cause });
    Object.assign(err, { code });
    return err;
  }
  var ClientError = class extends Error {
    code;
    constructor(message, options) {
      super(message, options);
      this.name = this.constructor.name;
      this.code = options?.code;
      Error.captureStackTrace?.(this, this.constructor);
    }
  };
  var decoder3 = new TextDecoder();
  function e(msg, cause, code) {
    return new ClientError(msg, { cause, code });
  }
  function errorHandler(err) {
    if (err instanceof TypeError || err instanceof ClientError || err instanceof ResponseBodyError || err instanceof AuthorizationResponseError || err instanceof WWWAuthenticateChallengeError) {
      throw err;
    }
    if (err instanceof OperationProcessingError) {
      switch (err.code) {
        case HTTP_REQUEST_FORBIDDEN:
          throw e("only requests to HTTPS are allowed", err, err.code);
        case REQUEST_PROTOCOL_FORBIDDEN:
          throw e("only requests to HTTP or HTTPS are allowed", err, err.code);
        case RESPONSE_IS_NOT_CONFORM:
          throw e("unexpected HTTP response status code", err.cause, err.code);
        case RESPONSE_IS_NOT_JSON:
          throw e("unexpected response content-type", err.cause, err.code);
        case PARSE_ERROR:
          throw e("parsing error occured", err, err.code);
        case INVALID_RESPONSE:
          throw e("invalid response encountered", err, err.code);
        case JWT_CLAIM_COMPARISON:
          throw e("unexpected JWT claim value encountered", err, err.code);
        case JSON_ATTRIBUTE_COMPARISON:
          throw e("unexpected JSON attribute value encountered", err, err.code);
        case JWT_TIMESTAMP_CHECK:
          throw e("JWT timestamp claim value failed validation", err, err.code);
        default:
          throw e(err.message, err, err.code);
      }
    }
    if (err instanceof UnsupportedOperationError) {
      throw e("unsupported operation", err, err.code);
    }
    if (err instanceof DOMException) {
      switch (err.name) {
        case "OperationError":
          throw e("runtime operation error", err, UNSUPPORTED_OPERATION);
        case "NotSupportedError":
          throw e("runtime unsupported operation", err, UNSUPPORTED_OPERATION);
        case "TimeoutError":
          throw e("operation timed out", err, "OAUTH_TIMEOUT");
        case "AbortError":
          throw e("operation aborted", err, "OAUTH_ABORT");
      }
    }
    throw new ClientError("something went wrong", { cause: err });
  }
  function handleEntraId(server, as, options) {
    if (server.origin === "https://login.microsoftonline.com" && (!options?.algorithm || options.algorithm === "oidc")) {
      as[kEntraId] = true;
      return true;
    }
    return false;
  }
  function handleB2Clogin(server, options) {
    if (server.hostname.endsWith(".b2clogin.com") && (!options?.algorithm || options.algorithm === "oidc")) {
      return true;
    }
    return false;
  }
  async function discovery(server, clientId, metadata, clientAuthentication, options) {
    const as = await performDiscovery2(server, options);
    const instance = new Configuration(as, clientId, metadata, clientAuthentication);
    let internals = int(instance);
    if (options?.[customFetch2]) {
      internals.fetch = options[customFetch2];
    }
    if (options?.timeout !== void 0) {
      internals.timeout = options.timeout;
    }
    if (options?.execute) {
      for (const extension of options.execute) {
        extension(instance);
      }
    }
    return instance;
  }
  async function performDiscovery2(server, options) {
    if (!(server instanceof URL)) {
      throw CodedTypeError2('"server" must be an instance of URL', ERR_INVALID_ARG_TYPE2);
    }
    const resolve = !server.href.includes("/.well-known/");
    const timeout = options?.timeout ?? 30;
    const signal3 = AbortSignal.timeout(timeout * 1e3);
    const as = await (resolve ? discoveryRequest(server, {
      algorithm: options?.algorithm,
      [customFetch]: options?.[customFetch2],
      [allowInsecureRequests]: options?.execute?.includes(allowInsecureRequests2),
      signal: signal3,
      headers: new Headers(headers)
    }) : (options?.[customFetch2] || fetch)((() => {
      checkProtocol(server, options?.execute?.includes(allowInsecureRequests2) ? false : true);
      return server.href;
    })(), {
      headers: Object.fromEntries(new Headers({ accept: "application/json", ...headers }).entries()),
      body: void 0,
      method: "GET",
      redirect: "manual",
      signal: signal3
    })).then((response) => processDiscoveryResponse(_nodiscoverycheck, response)).catch(errorHandler);
    if (resolve && new URL(as.issuer).href !== server.href) {
      handleEntraId(server, as, options) || handleB2Clogin(server, options) || (() => {
        throw new ClientError("discovered metadata issuer does not match the expected issuer", {
          code: JSON_ATTRIBUTE_COMPARISON,
          cause: {
            expected: server.href,
            body: as,
            attribute: "issuer"
          }
        });
      })();
    }
    return as;
  }
  function getServerHelpers(metadata) {
    return {
      supportsPKCE: {
        __proto__: null,
        value(method = "S256") {
          return metadata.code_challenge_methods_supported?.includes(method) === true;
        }
      }
    };
  }
  function addServerHelpers(metadata) {
    Object.defineProperties(metadata, getServerHelpers(metadata));
  }
  var kEntraId = Symbol();
  var Configuration = class {
    constructor(server, clientId, metadata, clientAuthentication) {
      if (typeof clientId !== "string" || !clientId.length) {
        throw CodedTypeError2('"clientId" must be a non-empty string', ERR_INVALID_ARG_TYPE2);
      }
      if (typeof metadata === "string") {
        metadata = { client_secret: metadata };
      }
      if (metadata?.client_id !== void 0 && clientId !== metadata.client_id) {
        throw CodedTypeError2('"clientId" and "metadata.client_id" must be the same', ERR_INVALID_ARG_VALUE2);
      }
      const client = {
        ...structuredClone(metadata),
        client_id: clientId
      };
      client[clockSkew] = metadata?.[clockSkew] ?? 0;
      client[clockTolerance] = metadata?.[clockTolerance] ?? 30;
      let auth;
      if (clientAuthentication) {
        auth = clientAuthentication;
      } else {
        if (typeof client.client_secret === "string" && client.client_secret.length) {
          auth = ClientSecretPost2(client.client_secret);
        } else {
          auth = None2();
        }
      }
      let c = Object.freeze(client);
      const clone = structuredClone(server);
      if (kEntraId in server) {
        clone[_expectedIssuer] = ({ claims: { tid } }) => server.issuer.replace("{tenantid}", tid);
      }
      let as = Object.freeze(clone);
      props ||= /* @__PURE__ */ new WeakMap();
      props.set(this, {
        __proto__: null,
        as,
        c,
        auth,
        tlsOnly: true,
        timeout: 30,
        jwksCache: {}
      });
    }
    serverMetadata() {
      const metadata = structuredClone(int(this).as);
      addServerHelpers(metadata);
      return metadata;
    }
    clientMetadata() {
      const metadata = structuredClone(int(this).c);
      return metadata;
    }
    get timeout() {
      return int(this).timeout;
    }
    set timeout(value) {
      int(this).timeout = value;
    }
    get [customFetch2]() {
      return int(this).fetch;
    }
    set [customFetch2](value) {
      int(this).fetch = value;
    }
  };
  Object.freeze(Configuration.prototype);
  function getHelpers(response) {
    let exp = void 0;
    if (response.expires_in !== void 0) {
      exp = Date.now() + response.expires_in * 1e3;
    }
    return {
      expiresIn: {
        __proto__: null,
        value() {
          if (exp) {
            const now = Date.now();
            if (exp > now) {
              return Math.floor((exp - now) / 1e3);
            }
            return 0;
          }
          return void 0;
        }
      },
      claims: {
        __proto__: null,
        value() {
          return getValidatedIdTokenClaims(response);
        }
      }
    };
  }
  function addHelpers(response) {
    Object.defineProperties(response, getHelpers(response));
  }
  function getDPoPHandle(config, keyPair, options) {
    checkConfig(config);
    return DPoP(int(config).c, keyPair, options);
  }
  function allowInsecureRequests2(config) {
    int(config).tlsOnly = false;
  }
  async function refreshTokenGrant(config, refreshToken, parameters, options) {
    checkConfig(config);
    parameters = new URLSearchParams(parameters);
    const { as, c, auth, fetch: fetch2, tlsOnly, nonRepudiation, timeout, decrypt } = int(config);
    const response = await refreshTokenGrantRequest(as, c, auth, refreshToken, {
      [customFetch]: fetch2,
      [allowInsecureRequests]: !tlsOnly,
      additionalParameters: parameters,
      DPoP: options?.DPoP,
      headers: new Headers(headers),
      signal: signal2(timeout)
    }).catch(errorHandler);
    const p = processRefreshTokenResponse(as, c, response, {
      [jweDecrypt]: decrypt
    });
    let result;
    try {
      result = await p;
    } catch (err) {
      if (retryable(err, options)) {
        return refreshTokenGrant(config, refreshToken, parameters, {
          ...options,
          flag: retry
        });
      }
      errorHandler(err);
    }
    result.id_token && await nonRepudiation?.(response);
    addHelpers(result);
    return result;
  }
  function checkConfig(input) {
    if (!(input instanceof Configuration)) {
      throw CodedTypeError2('"config" must be an instance of Configuration', ERR_INVALID_ARG_TYPE2);
    }
    if (Object.getPrototypeOf(input) !== Configuration.prototype) {
      throw CodedTypeError2("subclassing Configuration is not allowed", ERR_INVALID_ARG_VALUE2);
    }
  }
  function signal2(timeout) {
    return timeout ? AbortSignal.timeout(timeout * 1e3) : void 0;
  }
  function retryable(err, options) {
    if (options?.DPoP && options.flag !== retry) {
      return isDPoPNonceError(err);
    }
    return false;
  }
  var retry = Symbol();

  // node_modules/@streamparser/json/dist/mjs/utils/utf-8.js
  var charset;
  (function(charset2) {
    charset2[charset2["BACKSPACE"] = 8] = "BACKSPACE";
    charset2[charset2["FORM_FEED"] = 12] = "FORM_FEED";
    charset2[charset2["NEWLINE"] = 10] = "NEWLINE";
    charset2[charset2["CARRIAGE_RETURN"] = 13] = "CARRIAGE_RETURN";
    charset2[charset2["TAB"] = 9] = "TAB";
    charset2[charset2["SPACE"] = 32] = "SPACE";
    charset2[charset2["EXCLAMATION_MARK"] = 33] = "EXCLAMATION_MARK";
    charset2[charset2["QUOTATION_MARK"] = 34] = "QUOTATION_MARK";
    charset2[charset2["NUMBER_SIGN"] = 35] = "NUMBER_SIGN";
    charset2[charset2["DOLLAR_SIGN"] = 36] = "DOLLAR_SIGN";
    charset2[charset2["PERCENT_SIGN"] = 37] = "PERCENT_SIGN";
    charset2[charset2["AMPERSAND"] = 38] = "AMPERSAND";
    charset2[charset2["APOSTROPHE"] = 39] = "APOSTROPHE";
    charset2[charset2["LEFT_PARENTHESIS"] = 40] = "LEFT_PARENTHESIS";
    charset2[charset2["RIGHT_PARENTHESIS"] = 41] = "RIGHT_PARENTHESIS";
    charset2[charset2["ASTERISK"] = 42] = "ASTERISK";
    charset2[charset2["PLUS_SIGN"] = 43] = "PLUS_SIGN";
    charset2[charset2["COMMA"] = 44] = "COMMA";
    charset2[charset2["HYPHEN_MINUS"] = 45] = "HYPHEN_MINUS";
    charset2[charset2["FULL_STOP"] = 46] = "FULL_STOP";
    charset2[charset2["SOLIDUS"] = 47] = "SOLIDUS";
    charset2[charset2["DIGIT_ZERO"] = 48] = "DIGIT_ZERO";
    charset2[charset2["DIGIT_ONE"] = 49] = "DIGIT_ONE";
    charset2[charset2["DIGIT_TWO"] = 50] = "DIGIT_TWO";
    charset2[charset2["DIGIT_THREE"] = 51] = "DIGIT_THREE";
    charset2[charset2["DIGIT_FOUR"] = 52] = "DIGIT_FOUR";
    charset2[charset2["DIGIT_FIVE"] = 53] = "DIGIT_FIVE";
    charset2[charset2["DIGIT_SIX"] = 54] = "DIGIT_SIX";
    charset2[charset2["DIGIT_SEVEN"] = 55] = "DIGIT_SEVEN";
    charset2[charset2["DIGIT_EIGHT"] = 56] = "DIGIT_EIGHT";
    charset2[charset2["DIGIT_NINE"] = 57] = "DIGIT_NINE";
    charset2[charset2["COLON"] = 58] = "COLON";
    charset2[charset2["SEMICOLON"] = 59] = "SEMICOLON";
    charset2[charset2["LESS_THAN_SIGN"] = 60] = "LESS_THAN_SIGN";
    charset2[charset2["EQUALS_SIGN"] = 61] = "EQUALS_SIGN";
    charset2[charset2["GREATER_THAN_SIGN"] = 62] = "GREATER_THAN_SIGN";
    charset2[charset2["QUESTION_MARK"] = 63] = "QUESTION_MARK";
    charset2[charset2["COMMERCIAL_AT"] = 64] = "COMMERCIAL_AT";
    charset2[charset2["LATIN_CAPITAL_LETTER_A"] = 65] = "LATIN_CAPITAL_LETTER_A";
    charset2[charset2["LATIN_CAPITAL_LETTER_B"] = 66] = "LATIN_CAPITAL_LETTER_B";
    charset2[charset2["LATIN_CAPITAL_LETTER_C"] = 67] = "LATIN_CAPITAL_LETTER_C";
    charset2[charset2["LATIN_CAPITAL_LETTER_D"] = 68] = "LATIN_CAPITAL_LETTER_D";
    charset2[charset2["LATIN_CAPITAL_LETTER_E"] = 69] = "LATIN_CAPITAL_LETTER_E";
    charset2[charset2["LATIN_CAPITAL_LETTER_F"] = 70] = "LATIN_CAPITAL_LETTER_F";
    charset2[charset2["LATIN_CAPITAL_LETTER_G"] = 71] = "LATIN_CAPITAL_LETTER_G";
    charset2[charset2["LATIN_CAPITAL_LETTER_H"] = 72] = "LATIN_CAPITAL_LETTER_H";
    charset2[charset2["LATIN_CAPITAL_LETTER_I"] = 73] = "LATIN_CAPITAL_LETTER_I";
    charset2[charset2["LATIN_CAPITAL_LETTER_J"] = 74] = "LATIN_CAPITAL_LETTER_J";
    charset2[charset2["LATIN_CAPITAL_LETTER_K"] = 75] = "LATIN_CAPITAL_LETTER_K";
    charset2[charset2["LATIN_CAPITAL_LETTER_L"] = 76] = "LATIN_CAPITAL_LETTER_L";
    charset2[charset2["LATIN_CAPITAL_LETTER_M"] = 77] = "LATIN_CAPITAL_LETTER_M";
    charset2[charset2["LATIN_CAPITAL_LETTER_N"] = 78] = "LATIN_CAPITAL_LETTER_N";
    charset2[charset2["LATIN_CAPITAL_LETTER_O"] = 79] = "LATIN_CAPITAL_LETTER_O";
    charset2[charset2["LATIN_CAPITAL_LETTER_P"] = 80] = "LATIN_CAPITAL_LETTER_P";
    charset2[charset2["LATIN_CAPITAL_LETTER_Q"] = 81] = "LATIN_CAPITAL_LETTER_Q";
    charset2[charset2["LATIN_CAPITAL_LETTER_R"] = 82] = "LATIN_CAPITAL_LETTER_R";
    charset2[charset2["LATIN_CAPITAL_LETTER_S"] = 83] = "LATIN_CAPITAL_LETTER_S";
    charset2[charset2["LATIN_CAPITAL_LETTER_T"] = 84] = "LATIN_CAPITAL_LETTER_T";
    charset2[charset2["LATIN_CAPITAL_LETTER_U"] = 85] = "LATIN_CAPITAL_LETTER_U";
    charset2[charset2["LATIN_CAPITAL_LETTER_V"] = 86] = "LATIN_CAPITAL_LETTER_V";
    charset2[charset2["LATIN_CAPITAL_LETTER_W"] = 87] = "LATIN_CAPITAL_LETTER_W";
    charset2[charset2["LATIN_CAPITAL_LETTER_X"] = 88] = "LATIN_CAPITAL_LETTER_X";
    charset2[charset2["LATIN_CAPITAL_LETTER_Y"] = 89] = "LATIN_CAPITAL_LETTER_Y";
    charset2[charset2["LATIN_CAPITAL_LETTER_Z"] = 90] = "LATIN_CAPITAL_LETTER_Z";
    charset2[charset2["LEFT_SQUARE_BRACKET"] = 91] = "LEFT_SQUARE_BRACKET";
    charset2[charset2["REVERSE_SOLIDUS"] = 92] = "REVERSE_SOLIDUS";
    charset2[charset2["RIGHT_SQUARE_BRACKET"] = 93] = "RIGHT_SQUARE_BRACKET";
    charset2[charset2["CIRCUMFLEX_ACCENT"] = 94] = "CIRCUMFLEX_ACCENT";
    charset2[charset2["LOW_LINE"] = 95] = "LOW_LINE";
    charset2[charset2["GRAVE_ACCENT"] = 96] = "GRAVE_ACCENT";
    charset2[charset2["LATIN_SMALL_LETTER_A"] = 97] = "LATIN_SMALL_LETTER_A";
    charset2[charset2["LATIN_SMALL_LETTER_B"] = 98] = "LATIN_SMALL_LETTER_B";
    charset2[charset2["LATIN_SMALL_LETTER_C"] = 99] = "LATIN_SMALL_LETTER_C";
    charset2[charset2["LATIN_SMALL_LETTER_D"] = 100] = "LATIN_SMALL_LETTER_D";
    charset2[charset2["LATIN_SMALL_LETTER_E"] = 101] = "LATIN_SMALL_LETTER_E";
    charset2[charset2["LATIN_SMALL_LETTER_F"] = 102] = "LATIN_SMALL_LETTER_F";
    charset2[charset2["LATIN_SMALL_LETTER_G"] = 103] = "LATIN_SMALL_LETTER_G";
    charset2[charset2["LATIN_SMALL_LETTER_H"] = 104] = "LATIN_SMALL_LETTER_H";
    charset2[charset2["LATIN_SMALL_LETTER_I"] = 105] = "LATIN_SMALL_LETTER_I";
    charset2[charset2["LATIN_SMALL_LETTER_J"] = 106] = "LATIN_SMALL_LETTER_J";
    charset2[charset2["LATIN_SMALL_LETTER_K"] = 107] = "LATIN_SMALL_LETTER_K";
    charset2[charset2["LATIN_SMALL_LETTER_L"] = 108] = "LATIN_SMALL_LETTER_L";
    charset2[charset2["LATIN_SMALL_LETTER_M"] = 109] = "LATIN_SMALL_LETTER_M";
    charset2[charset2["LATIN_SMALL_LETTER_N"] = 110] = "LATIN_SMALL_LETTER_N";
    charset2[charset2["LATIN_SMALL_LETTER_O"] = 111] = "LATIN_SMALL_LETTER_O";
    charset2[charset2["LATIN_SMALL_LETTER_P"] = 112] = "LATIN_SMALL_LETTER_P";
    charset2[charset2["LATIN_SMALL_LETTER_Q"] = 113] = "LATIN_SMALL_LETTER_Q";
    charset2[charset2["LATIN_SMALL_LETTER_R"] = 114] = "LATIN_SMALL_LETTER_R";
    charset2[charset2["LATIN_SMALL_LETTER_S"] = 115] = "LATIN_SMALL_LETTER_S";
    charset2[charset2["LATIN_SMALL_LETTER_T"] = 116] = "LATIN_SMALL_LETTER_T";
    charset2[charset2["LATIN_SMALL_LETTER_U"] = 117] = "LATIN_SMALL_LETTER_U";
    charset2[charset2["LATIN_SMALL_LETTER_V"] = 118] = "LATIN_SMALL_LETTER_V";
    charset2[charset2["LATIN_SMALL_LETTER_W"] = 119] = "LATIN_SMALL_LETTER_W";
    charset2[charset2["LATIN_SMALL_LETTER_X"] = 120] = "LATIN_SMALL_LETTER_X";
    charset2[charset2["LATIN_SMALL_LETTER_Y"] = 121] = "LATIN_SMALL_LETTER_Y";
    charset2[charset2["LATIN_SMALL_LETTER_Z"] = 122] = "LATIN_SMALL_LETTER_Z";
    charset2[charset2["LEFT_CURLY_BRACKET"] = 123] = "LEFT_CURLY_BRACKET";
    charset2[charset2["VERTICAL_LINE"] = 124] = "VERTICAL_LINE";
    charset2[charset2["RIGHT_CURLY_BRACKET"] = 125] = "RIGHT_CURLY_BRACKET";
    charset2[charset2["TILDE"] = 126] = "TILDE";
  })(charset || (charset = {}));
  var escapedSequences = {
    [charset.QUOTATION_MARK]: charset.QUOTATION_MARK,
    [charset.REVERSE_SOLIDUS]: charset.REVERSE_SOLIDUS,
    [charset.SOLIDUS]: charset.SOLIDUS,
    [charset.LATIN_SMALL_LETTER_B]: charset.BACKSPACE,
    [charset.LATIN_SMALL_LETTER_F]: charset.FORM_FEED,
    [charset.LATIN_SMALL_LETTER_N]: charset.NEWLINE,
    [charset.LATIN_SMALL_LETTER_R]: charset.CARRIAGE_RETURN,
    [charset.LATIN_SMALL_LETTER_T]: charset.TAB
  };

  // node_modules/@streamparser/json/dist/mjs/utils/bufferedString.js
  var NonBufferedString = class {
    constructor() {
      this.decoder = new TextDecoder("utf-8");
      this.strings = [];
      this.byteLength = 0;
    }
    appendChar(char) {
      this.strings.push(String.fromCharCode(char));
      this.byteLength += 1;
    }
    appendBuf(buf2, start = 0, end = buf2.length) {
      this.strings.push(this.decoder.decode(buf2.subarray(start, end)));
      this.byteLength += end - start;
    }
    reset() {
      this.strings = [];
      this.byteLength = 0;
    }
    toString() {
      return this.strings.join("");
    }
  };
  var BufferedString = class {
    constructor(bufferSize) {
      this.decoder = new TextDecoder("utf-8");
      this.bufferOffset = 0;
      this.string = "";
      this.byteLength = 0;
      this.buffer = new Uint8Array(bufferSize);
    }
    appendChar(char) {
      if (this.bufferOffset >= this.buffer.length)
        this.flushStringBuffer();
      this.buffer[this.bufferOffset++] = char;
      this.byteLength += 1;
    }
    appendBuf(buf2, start = 0, end = buf2.length) {
      const size = end - start;
      if (this.bufferOffset + size > this.buffer.length)
        this.flushStringBuffer();
      this.buffer.set(buf2.subarray(start, end), this.bufferOffset);
      this.bufferOffset += size;
      this.byteLength += size;
    }
    flushStringBuffer() {
      this.string += this.decoder.decode(this.buffer.subarray(0, this.bufferOffset));
      this.bufferOffset = 0;
    }
    reset() {
      this.string = "";
      this.bufferOffset = 0;
      this.byteLength = 0;
    }
    toString() {
      this.flushStringBuffer();
      return this.string;
    }
  };

  // node_modules/@streamparser/json/dist/mjs/utils/types/tokenType.js
  var TokenType;
  (function(TokenType2) {
    TokenType2[TokenType2["LEFT_BRACE"] = 0] = "LEFT_BRACE";
    TokenType2[TokenType2["RIGHT_BRACE"] = 1] = "RIGHT_BRACE";
    TokenType2[TokenType2["LEFT_BRACKET"] = 2] = "LEFT_BRACKET";
    TokenType2[TokenType2["RIGHT_BRACKET"] = 3] = "RIGHT_BRACKET";
    TokenType2[TokenType2["COLON"] = 4] = "COLON";
    TokenType2[TokenType2["COMMA"] = 5] = "COMMA";
    TokenType2[TokenType2["TRUE"] = 6] = "TRUE";
    TokenType2[TokenType2["FALSE"] = 7] = "FALSE";
    TokenType2[TokenType2["NULL"] = 8] = "NULL";
    TokenType2[TokenType2["STRING"] = 9] = "STRING";
    TokenType2[TokenType2["NUMBER"] = 10] = "NUMBER";
    TokenType2[TokenType2["SEPARATOR"] = 11] = "SEPARATOR";
  })(TokenType || (TokenType = {}));
  var tokenType_default = TokenType;

  // node_modules/@streamparser/json/dist/mjs/tokenizer.js
  var TokenizerStates;
  (function(TokenizerStates2) {
    TokenizerStates2[TokenizerStates2["START"] = 0] = "START";
    TokenizerStates2[TokenizerStates2["ENDED"] = 1] = "ENDED";
    TokenizerStates2[TokenizerStates2["ERROR"] = 2] = "ERROR";
    TokenizerStates2[TokenizerStates2["TRUE1"] = 3] = "TRUE1";
    TokenizerStates2[TokenizerStates2["TRUE2"] = 4] = "TRUE2";
    TokenizerStates2[TokenizerStates2["TRUE3"] = 5] = "TRUE3";
    TokenizerStates2[TokenizerStates2["FALSE1"] = 6] = "FALSE1";
    TokenizerStates2[TokenizerStates2["FALSE2"] = 7] = "FALSE2";
    TokenizerStates2[TokenizerStates2["FALSE3"] = 8] = "FALSE3";
    TokenizerStates2[TokenizerStates2["FALSE4"] = 9] = "FALSE4";
    TokenizerStates2[TokenizerStates2["NULL1"] = 10] = "NULL1";
    TokenizerStates2[TokenizerStates2["NULL2"] = 11] = "NULL2";
    TokenizerStates2[TokenizerStates2["NULL3"] = 12] = "NULL3";
    TokenizerStates2[TokenizerStates2["STRING_DEFAULT"] = 13] = "STRING_DEFAULT";
    TokenizerStates2[TokenizerStates2["STRING_AFTER_BACKSLASH"] = 14] = "STRING_AFTER_BACKSLASH";
    TokenizerStates2[TokenizerStates2["STRING_UNICODE_DIGIT_1"] = 15] = "STRING_UNICODE_DIGIT_1";
    TokenizerStates2[TokenizerStates2["STRING_UNICODE_DIGIT_2"] = 16] = "STRING_UNICODE_DIGIT_2";
    TokenizerStates2[TokenizerStates2["STRING_UNICODE_DIGIT_3"] = 17] = "STRING_UNICODE_DIGIT_3";
    TokenizerStates2[TokenizerStates2["STRING_UNICODE_DIGIT_4"] = 18] = "STRING_UNICODE_DIGIT_4";
    TokenizerStates2[TokenizerStates2["STRING_INCOMPLETE_CHAR"] = 19] = "STRING_INCOMPLETE_CHAR";
    TokenizerStates2[TokenizerStates2["NUMBER_AFTER_INITIAL_MINUS"] = 20] = "NUMBER_AFTER_INITIAL_MINUS";
    TokenizerStates2[TokenizerStates2["NUMBER_AFTER_INITIAL_ZERO"] = 21] = "NUMBER_AFTER_INITIAL_ZERO";
    TokenizerStates2[TokenizerStates2["NUMBER_AFTER_INITIAL_NON_ZERO"] = 22] = "NUMBER_AFTER_INITIAL_NON_ZERO";
    TokenizerStates2[TokenizerStates2["NUMBER_AFTER_FULL_STOP"] = 23] = "NUMBER_AFTER_FULL_STOP";
    TokenizerStates2[TokenizerStates2["NUMBER_AFTER_DECIMAL"] = 24] = "NUMBER_AFTER_DECIMAL";
    TokenizerStates2[TokenizerStates2["NUMBER_AFTER_E"] = 25] = "NUMBER_AFTER_E";
    TokenizerStates2[TokenizerStates2["NUMBER_AFTER_E_AND_SIGN"] = 26] = "NUMBER_AFTER_E_AND_SIGN";
    TokenizerStates2[TokenizerStates2["NUMBER_AFTER_E_AND_DIGIT"] = 27] = "NUMBER_AFTER_E_AND_DIGIT";
    TokenizerStates2[TokenizerStates2["SEPARATOR"] = 28] = "SEPARATOR";
    TokenizerStates2[TokenizerStates2["BOM_OR_START"] = 29] = "BOM_OR_START";
    TokenizerStates2[TokenizerStates2["BOM"] = 30] = "BOM";
  })(TokenizerStates || (TokenizerStates = {}));
  function TokenizerStateToString(tokenizerState) {
    return [
      "START",
      "ENDED",
      "ERROR",
      "TRUE1",
      "TRUE2",
      "TRUE3",
      "FALSE1",
      "FALSE2",
      "FALSE3",
      "FALSE4",
      "NULL1",
      "NULL2",
      "NULL3",
      "STRING_DEFAULT",
      "STRING_AFTER_BACKSLASH",
      "STRING_UNICODE_DIGIT_1",
      "STRING_UNICODE_DIGIT_2",
      "STRING_UNICODE_DIGIT_3",
      "STRING_UNICODE_DIGIT_4",
      "STRING_INCOMPLETE_CHAR",
      "NUMBER_AFTER_INITIAL_MINUS",
      "NUMBER_AFTER_INITIAL_ZERO",
      "NUMBER_AFTER_INITIAL_NON_ZERO",
      "NUMBER_AFTER_FULL_STOP",
      "NUMBER_AFTER_DECIMAL",
      "NUMBER_AFTER_E",
      "NUMBER_AFTER_E_AND_SIGN",
      "NUMBER_AFTER_E_AND_DIGIT",
      "SEPARATOR",
      "BOM_OR_START",
      "BOM"
    ][tokenizerState];
  }
  var defaultOpts = {
    stringBufferSize: 0,
    numberBufferSize: 0,
    separator: void 0,
    emitPartialTokens: false
  };
  var TokenizerError = class _TokenizerError extends Error {
    constructor(message) {
      super(message);
      Object.setPrototypeOf(this, _TokenizerError.prototype);
    }
  };
  var Tokenizer = class {
    constructor(opts) {
      this.state = TokenizerStates.BOM_OR_START;
      this.bomIndex = 0;
      this.separatorIndex = 0;
      this.escapedCharsByteLength = 0;
      this.bytes_remaining = 0;
      this.bytes_in_sequence = 0;
      this.char_split_buffer = new Uint8Array(4);
      this.encoder = new TextEncoder();
      this.offset = -1;
      opts = Object.assign(Object.assign({}, defaultOpts), opts);
      this.emitPartialTokens = opts.emitPartialTokens === true;
      this.bufferedString = opts.stringBufferSize && opts.stringBufferSize > 4 ? new BufferedString(opts.stringBufferSize) : new NonBufferedString();
      this.bufferedNumber = opts.numberBufferSize && opts.numberBufferSize > 0 ? new BufferedString(opts.numberBufferSize) : new NonBufferedString();
      this.separator = opts.separator;
      this.separatorBytes = opts.separator ? this.encoder.encode(opts.separator) : void 0;
    }
    get isEnded() {
      return this.state === TokenizerStates.ENDED;
    }
    write(input) {
      try {
        let buffer;
        if (input instanceof Uint8Array) {
          buffer = input;
        } else if (typeof input === "string") {
          buffer = this.encoder.encode(input);
        } else if (Array.isArray(input)) {
          buffer = Uint8Array.from(input);
        } else if (ArrayBuffer.isView(input)) {
          buffer = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
        } else {
          throw new TypeError("Unexpected type. The `write` function only accepts Arrays, TypedArrays and Strings.");
        }
        for (let i = 0; i < buffer.length; i += 1) {
          const n = buffer[i];
          switch (this.state) {
            // @ts-expect-error fall through case
            case TokenizerStates.BOM_OR_START:
              if (input instanceof Uint8Array && n === 239) {
                this.bom = [239, 187, 191];
                this.bomIndex += 1;
                this.state = TokenizerStates.BOM;
                continue;
              }
              if (input instanceof Uint16Array) {
                if (n === 254) {
                  this.bom = [254, 255];
                  this.bomIndex += 1;
                  this.state = TokenizerStates.BOM;
                  continue;
                }
                if (n === 255) {
                  this.bom = [255, 254];
                  this.bomIndex += 1;
                  this.state = TokenizerStates.BOM;
                  continue;
                }
              }
              if (input instanceof Uint32Array) {
                if (n === 0) {
                  this.bom = [0, 0, 254, 255];
                  this.bomIndex += 1;
                  this.state = TokenizerStates.BOM;
                  continue;
                }
                if (n === 255) {
                  this.bom = [255, 254, 0, 0];
                  this.bomIndex += 1;
                  this.state = TokenizerStates.BOM;
                  continue;
                }
              }
            // eslint-disable-next-line no-fallthrough
            case TokenizerStates.START:
              this.offset += 1;
              if (this.separatorBytes && n === this.separatorBytes[0]) {
                if (this.separatorBytes.length === 1) {
                  this.state = TokenizerStates.START;
                  this.onToken({
                    token: tokenType_default.SEPARATOR,
                    value: this.separator,
                    offset: this.offset + this.separatorBytes.length - 1
                  });
                  continue;
                }
                this.state = TokenizerStates.SEPARATOR;
                continue;
              }
              if (n === charset.SPACE || n === charset.NEWLINE || n === charset.CARRIAGE_RETURN || n === charset.TAB) {
                continue;
              }
              if (n === charset.LEFT_CURLY_BRACKET) {
                this.onToken({
                  token: tokenType_default.LEFT_BRACE,
                  value: "{",
                  offset: this.offset
                });
                continue;
              }
              if (n === charset.RIGHT_CURLY_BRACKET) {
                this.onToken({
                  token: tokenType_default.RIGHT_BRACE,
                  value: "}",
                  offset: this.offset
                });
                continue;
              }
              if (n === charset.LEFT_SQUARE_BRACKET) {
                this.onToken({
                  token: tokenType_default.LEFT_BRACKET,
                  value: "[",
                  offset: this.offset
                });
                continue;
              }
              if (n === charset.RIGHT_SQUARE_BRACKET) {
                this.onToken({
                  token: tokenType_default.RIGHT_BRACKET,
                  value: "]",
                  offset: this.offset
                });
                continue;
              }
              if (n === charset.COLON) {
                this.onToken({
                  token: tokenType_default.COLON,
                  value: ":",
                  offset: this.offset
                });
                continue;
              }
              if (n === charset.COMMA) {
                this.onToken({
                  token: tokenType_default.COMMA,
                  value: ",",
                  offset: this.offset
                });
                continue;
              }
              if (n === charset.LATIN_SMALL_LETTER_T) {
                this.state = TokenizerStates.TRUE1;
                continue;
              }
              if (n === charset.LATIN_SMALL_LETTER_F) {
                this.state = TokenizerStates.FALSE1;
                continue;
              }
              if (n === charset.LATIN_SMALL_LETTER_N) {
                this.state = TokenizerStates.NULL1;
                continue;
              }
              if (n === charset.QUOTATION_MARK) {
                this.bufferedString.reset();
                this.escapedCharsByteLength = 0;
                this.state = TokenizerStates.STRING_DEFAULT;
                continue;
              }
              if (n >= charset.DIGIT_ONE && n <= charset.DIGIT_NINE) {
                this.bufferedNumber.reset();
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_INITIAL_NON_ZERO;
                continue;
              }
              if (n === charset.DIGIT_ZERO) {
                this.bufferedNumber.reset();
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_INITIAL_ZERO;
                continue;
              }
              if (n === charset.HYPHEN_MINUS) {
                this.bufferedNumber.reset();
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_INITIAL_MINUS;
                continue;
              }
              break;
            // STRING
            case TokenizerStates.STRING_DEFAULT:
              if (n === charset.QUOTATION_MARK) {
                const string = this.bufferedString.toString();
                this.state = TokenizerStates.START;
                this.onToken({
                  token: tokenType_default.STRING,
                  value: string,
                  offset: this.offset
                });
                this.offset += this.escapedCharsByteLength + this.bufferedString.byteLength + 1;
                continue;
              }
              if (n === charset.REVERSE_SOLIDUS) {
                this.state = TokenizerStates.STRING_AFTER_BACKSLASH;
                continue;
              }
              if (n >= 128) {
                if (n >= 194 && n <= 223) {
                  this.bytes_in_sequence = 2;
                } else if (n <= 239) {
                  this.bytes_in_sequence = 3;
                } else {
                  this.bytes_in_sequence = 4;
                }
                if (this.bytes_in_sequence <= buffer.length - i) {
                  this.bufferedString.appendBuf(buffer, i, i + this.bytes_in_sequence);
                  i += this.bytes_in_sequence - 1;
                  continue;
                }
                this.bytes_remaining = i + this.bytes_in_sequence - buffer.length;
                this.char_split_buffer.set(buffer.subarray(i));
                i = buffer.length - 1;
                this.state = TokenizerStates.STRING_INCOMPLETE_CHAR;
                continue;
              }
              if (n >= charset.SPACE) {
                this.bufferedString.appendChar(n);
                continue;
              }
              break;
            case TokenizerStates.STRING_INCOMPLETE_CHAR:
              this.char_split_buffer.set(buffer.subarray(i, i + this.bytes_remaining), this.bytes_in_sequence - this.bytes_remaining);
              this.bufferedString.appendBuf(this.char_split_buffer, 0, this.bytes_in_sequence);
              i = this.bytes_remaining - 1;
              this.state = TokenizerStates.STRING_DEFAULT;
              continue;
            case TokenizerStates.STRING_AFTER_BACKSLASH:
              const controlChar = escapedSequences[n];
              if (controlChar) {
                this.bufferedString.appendChar(controlChar);
                this.escapedCharsByteLength += 1;
                this.state = TokenizerStates.STRING_DEFAULT;
                continue;
              }
              if (n === charset.LATIN_SMALL_LETTER_U) {
                this.unicode = "";
                this.state = TokenizerStates.STRING_UNICODE_DIGIT_1;
                continue;
              }
              break;
            case TokenizerStates.STRING_UNICODE_DIGIT_1:
            case TokenizerStates.STRING_UNICODE_DIGIT_2:
            case TokenizerStates.STRING_UNICODE_DIGIT_3:
              if (n >= charset.DIGIT_ZERO && n <= charset.DIGIT_NINE || n >= charset.LATIN_CAPITAL_LETTER_A && n <= charset.LATIN_CAPITAL_LETTER_F || n >= charset.LATIN_SMALL_LETTER_A && n <= charset.LATIN_SMALL_LETTER_F) {
                this.unicode += String.fromCharCode(n);
                this.state += 1;
                continue;
              }
              break;
            case TokenizerStates.STRING_UNICODE_DIGIT_4:
              if (n >= charset.DIGIT_ZERO && n <= charset.DIGIT_NINE || n >= charset.LATIN_CAPITAL_LETTER_A && n <= charset.LATIN_CAPITAL_LETTER_F || n >= charset.LATIN_SMALL_LETTER_A && n <= charset.LATIN_SMALL_LETTER_F) {
                const intVal = parseInt(this.unicode + String.fromCharCode(n), 16);
                let unicodeString;
                if (this.highSurrogate === void 0) {
                  if (intVal >= 55296 && intVal <= 56319) {
                    this.highSurrogate = intVal;
                    this.state = TokenizerStates.STRING_DEFAULT;
                    continue;
                  } else {
                    unicodeString = String.fromCharCode(intVal);
                  }
                } else {
                  if (intVal >= 56320 && intVal <= 57343) {
                    unicodeString = String.fromCharCode(this.highSurrogate, intVal);
                  } else {
                    unicodeString = String.fromCharCode(this.highSurrogate);
                  }
                  this.highSurrogate = void 0;
                }
                const unicodeBuffer = this.encoder.encode(unicodeString);
                this.bufferedString.appendBuf(unicodeBuffer);
                this.escapedCharsByteLength += 6 - unicodeBuffer.byteLength;
                this.state = TokenizerStates.STRING_DEFAULT;
                continue;
              }
              break;
            // Number
            case TokenizerStates.NUMBER_AFTER_INITIAL_MINUS:
              if (n === charset.DIGIT_ZERO) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_INITIAL_ZERO;
                continue;
              }
              if (n >= charset.DIGIT_ONE && n <= charset.DIGIT_NINE) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_INITIAL_NON_ZERO;
                continue;
              }
              break;
            case TokenizerStates.NUMBER_AFTER_INITIAL_ZERO:
              if (n === charset.FULL_STOP) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_FULL_STOP;
                continue;
              }
              if (n === charset.LATIN_SMALL_LETTER_E || n === charset.LATIN_CAPITAL_LETTER_E) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_E;
                continue;
              }
              i -= 1;
              this.state = TokenizerStates.START;
              this.emitNumber();
              continue;
            case TokenizerStates.NUMBER_AFTER_INITIAL_NON_ZERO:
              if (n >= charset.DIGIT_ZERO && n <= charset.DIGIT_NINE) {
                this.bufferedNumber.appendChar(n);
                continue;
              }
              if (n === charset.FULL_STOP) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_FULL_STOP;
                continue;
              }
              if (n === charset.LATIN_SMALL_LETTER_E || n === charset.LATIN_CAPITAL_LETTER_E) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_E;
                continue;
              }
              i -= 1;
              this.state = TokenizerStates.START;
              this.emitNumber();
              continue;
            case TokenizerStates.NUMBER_AFTER_FULL_STOP:
              if (n >= charset.DIGIT_ZERO && n <= charset.DIGIT_NINE) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_DECIMAL;
                continue;
              }
              break;
            case TokenizerStates.NUMBER_AFTER_DECIMAL:
              if (n >= charset.DIGIT_ZERO && n <= charset.DIGIT_NINE) {
                this.bufferedNumber.appendChar(n);
                continue;
              }
              if (n === charset.LATIN_SMALL_LETTER_E || n === charset.LATIN_CAPITAL_LETTER_E) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_E;
                continue;
              }
              i -= 1;
              this.state = TokenizerStates.START;
              this.emitNumber();
              continue;
            // @ts-expect-error fall through case
            case TokenizerStates.NUMBER_AFTER_E:
              if (n === charset.PLUS_SIGN || n === charset.HYPHEN_MINUS) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_E_AND_SIGN;
                continue;
              }
            // eslint-disable-next-line no-fallthrough
            case TokenizerStates.NUMBER_AFTER_E_AND_SIGN:
              if (n >= charset.DIGIT_ZERO && n <= charset.DIGIT_NINE) {
                this.bufferedNumber.appendChar(n);
                this.state = TokenizerStates.NUMBER_AFTER_E_AND_DIGIT;
                continue;
              }
              break;
            case TokenizerStates.NUMBER_AFTER_E_AND_DIGIT:
              if (n >= charset.DIGIT_ZERO && n <= charset.DIGIT_NINE) {
                this.bufferedNumber.appendChar(n);
                continue;
              }
              i -= 1;
              this.state = TokenizerStates.START;
              this.emitNumber();
              continue;
            // TRUE
            case TokenizerStates.TRUE1:
              if (n === charset.LATIN_SMALL_LETTER_R) {
                this.state = TokenizerStates.TRUE2;
                continue;
              }
              break;
            case TokenizerStates.TRUE2:
              if (n === charset.LATIN_SMALL_LETTER_U) {
                this.state = TokenizerStates.TRUE3;
                continue;
              }
              break;
            case TokenizerStates.TRUE3:
              if (n === charset.LATIN_SMALL_LETTER_E) {
                this.state = TokenizerStates.START;
                this.onToken({
                  token: tokenType_default.TRUE,
                  value: true,
                  offset: this.offset
                });
                this.offset += 3;
                continue;
              }
              break;
            // FALSE
            case TokenizerStates.FALSE1:
              if (n === charset.LATIN_SMALL_LETTER_A) {
                this.state = TokenizerStates.FALSE2;
                continue;
              }
              break;
            case TokenizerStates.FALSE2:
              if (n === charset.LATIN_SMALL_LETTER_L) {
                this.state = TokenizerStates.FALSE3;
                continue;
              }
              break;
            case TokenizerStates.FALSE3:
              if (n === charset.LATIN_SMALL_LETTER_S) {
                this.state = TokenizerStates.FALSE4;
                continue;
              }
              break;
            case TokenizerStates.FALSE4:
              if (n === charset.LATIN_SMALL_LETTER_E) {
                this.state = TokenizerStates.START;
                this.onToken({
                  token: tokenType_default.FALSE,
                  value: false,
                  offset: this.offset
                });
                this.offset += 4;
                continue;
              }
              break;
            // NULL
            case TokenizerStates.NULL1:
              if (n === charset.LATIN_SMALL_LETTER_U) {
                this.state = TokenizerStates.NULL2;
                continue;
              }
              break;
            case TokenizerStates.NULL2:
              if (n === charset.LATIN_SMALL_LETTER_L) {
                this.state = TokenizerStates.NULL3;
                continue;
              }
              break;
            case TokenizerStates.NULL3:
              if (n === charset.LATIN_SMALL_LETTER_L) {
                this.state = TokenizerStates.START;
                this.onToken({
                  token: tokenType_default.NULL,
                  value: null,
                  offset: this.offset
                });
                this.offset += 3;
                continue;
              }
              break;
            case TokenizerStates.SEPARATOR:
              this.separatorIndex += 1;
              if (!this.separatorBytes || n !== this.separatorBytes[this.separatorIndex]) {
                break;
              }
              if (this.separatorIndex === this.separatorBytes.length - 1) {
                this.state = TokenizerStates.START;
                this.onToken({
                  token: tokenType_default.SEPARATOR,
                  value: this.separator,
                  offset: this.offset + this.separatorIndex
                });
                this.separatorIndex = 0;
              }
              continue;
            // BOM support
            case TokenizerStates.BOM:
              if (n === this.bom[this.bomIndex]) {
                if (this.bomIndex === this.bom.length - 1) {
                  this.state = TokenizerStates.START;
                  this.bom = void 0;
                  this.bomIndex = 0;
                  continue;
                }
                this.bomIndex += 1;
                continue;
              }
              break;
            case TokenizerStates.ENDED:
              if (n === charset.SPACE || n === charset.NEWLINE || n === charset.CARRIAGE_RETURN || n === charset.TAB) {
                continue;
              }
          }
          throw new TokenizerError(`Unexpected "${String.fromCharCode(n)}" at position "${i}" in state ${TokenizerStateToString(this.state)}`);
        }
        if (this.emitPartialTokens) {
          switch (this.state) {
            case TokenizerStates.TRUE1:
            case TokenizerStates.TRUE2:
            case TokenizerStates.TRUE3:
              this.onToken({
                token: tokenType_default.TRUE,
                value: true,
                offset: this.offset,
                partial: true
              });
              break;
            case TokenizerStates.FALSE1:
            case TokenizerStates.FALSE2:
            case TokenizerStates.FALSE3:
            case TokenizerStates.FALSE4:
              this.onToken({
                token: tokenType_default.FALSE,
                value: false,
                offset: this.offset,
                partial: true
              });
              break;
            case TokenizerStates.NULL1:
            case TokenizerStates.NULL2:
            case TokenizerStates.NULL3:
              this.onToken({
                token: tokenType_default.NULL,
                value: null,
                offset: this.offset,
                partial: true
              });
              break;
            case TokenizerStates.STRING_DEFAULT: {
              const string = this.bufferedString.toString();
              this.onToken({
                token: tokenType_default.STRING,
                value: string,
                offset: this.offset,
                partial: true
              });
              break;
            }
            case TokenizerStates.NUMBER_AFTER_INITIAL_ZERO:
            case TokenizerStates.NUMBER_AFTER_INITIAL_NON_ZERO:
            case TokenizerStates.NUMBER_AFTER_DECIMAL:
            case TokenizerStates.NUMBER_AFTER_E_AND_DIGIT:
              try {
                this.onToken({
                  token: tokenType_default.NUMBER,
                  value: this.parseNumber(this.bufferedNumber.toString()),
                  offset: this.offset,
                  partial: true
                });
              } catch (_a) {
              }
          }
        }
      } catch (err) {
        this.error(err);
      }
    }
    emitNumber() {
      this.onToken({
        token: tokenType_default.NUMBER,
        value: this.parseNumber(this.bufferedNumber.toString()),
        offset: this.offset
      });
      this.offset += this.bufferedNumber.byteLength - 1;
    }
    parseNumber(numberStr) {
      return Number(numberStr);
    }
    error(err) {
      if (this.state !== TokenizerStates.ENDED) {
        this.state = TokenizerStates.ERROR;
      }
      this.onError(err);
    }
    end() {
      switch (this.state) {
        case TokenizerStates.NUMBER_AFTER_INITIAL_ZERO:
        case TokenizerStates.NUMBER_AFTER_INITIAL_NON_ZERO:
        case TokenizerStates.NUMBER_AFTER_DECIMAL:
        case TokenizerStates.NUMBER_AFTER_E_AND_DIGIT:
          this.state = TokenizerStates.ENDED;
          this.emitNumber();
          this.onEnd();
          break;
        case TokenizerStates.BOM_OR_START:
        case TokenizerStates.START:
        case TokenizerStates.ERROR:
        case TokenizerStates.SEPARATOR:
          this.state = TokenizerStates.ENDED;
          this.onEnd();
          break;
        default:
          this.error(new TokenizerError(`Tokenizer ended in the middle of a token (state: ${TokenizerStateToString(this.state)}). Either not all the data was received or the data was invalid.`));
      }
    }
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    onToken(parsedToken) {
      throw new TokenizerError(`Can't emit tokens before the "onToken" callback has been set up.`);
    }
    onError(err) {
      throw err;
    }
    onEnd() {
    }
  };

  // node_modules/@streamparser/json/dist/mjs/utils/types/stackElement.js
  var TokenParserMode;
  (function(TokenParserMode2) {
    TokenParserMode2[TokenParserMode2["OBJECT"] = 0] = "OBJECT";
    TokenParserMode2[TokenParserMode2["ARRAY"] = 1] = "ARRAY";
  })(TokenParserMode || (TokenParserMode = {}));

  // node_modules/@streamparser/json/dist/mjs/tokenparser.js
  var TokenParserState;
  (function(TokenParserState2) {
    TokenParserState2[TokenParserState2["VALUE"] = 0] = "VALUE";
    TokenParserState2[TokenParserState2["KEY"] = 1] = "KEY";
    TokenParserState2[TokenParserState2["COLON"] = 2] = "COLON";
    TokenParserState2[TokenParserState2["COMMA"] = 3] = "COMMA";
    TokenParserState2[TokenParserState2["ENDED"] = 4] = "ENDED";
    TokenParserState2[TokenParserState2["ERROR"] = 5] = "ERROR";
    TokenParserState2[TokenParserState2["SEPARATOR"] = 6] = "SEPARATOR";
  })(TokenParserState || (TokenParserState = {}));

  // src/scan.mjs
  var COLD_HEADER = "COLDSTORAGE";
  var UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  var MAX_STRING = 64 * 1024 * 1024;
  var MAX_BLOCK = 1024 * 1024 * 1024;
  var MAX_REFERENCE = 64 * 1024;
  var MAX_HELD_PLUGIN_STORAGE = 64 * 1024;
  var EMPTY_PLUGIN_STORAGE = Uint8Array.of(11, 0, 13, ...encoder.encode("pluginStorage"), 2, 0, 0, 0, ...encoder.encode("{}"));
  var JsonScan = class {
    constructor(visit, { maxString = MAX_STRING, blank = false, cold = false } = {}) {
      this.visit = visit;
      this.stack = [];
      this.rootDone = false;
      this.pending = 0;
      this.maxString = maxString;
      this.blank = blank;
      this.cold = cold;
      this.receivedBytes = 0;
      this.previousScalar = null;
      this.decoder = new TextDecoder("utf-8", { fatal: true });
      this.tokenizer = new Tokenizer({ stringBufferSize: 64 * 1024 });
      this.tokenizer.onToken = (token) => {
        this.pending = 0;
        this.token(token);
      };
    }
    token({ token, value, offset }) {
      this.checkScalarEnd(offset);
      if (token === tokenType_default.STRING && encoder.encode(JSON.stringify(value)).length > this.maxString) fail("recordTooLarge");
      if ([tokenType_default.STRING, tokenType_default.NUMBER, tokenType_default.TRUE, tokenType_default.FALSE, tokenType_default.NULL].includes(token)) this.previousScalar = offset;
      for (const container of this.stack) {
        if (container.bounded && offset - container.start + 1 > this.maxString) fail("recordTooLarge");
      }
      const frame = this.stack.at(-1);
      if (frame?.state === "colon") {
        if (token !== tokenType_default.COLON) fail("invalidData");
        frame.state = "value";
        frame.valueStart = offset + 1;
        return;
      }
      if (frame?.state === "comma") {
        if (token === tokenType_default.COMMA) {
          frame.state = frame.object ? "key" : "value";
          return;
        }
        if (token === (frame.object ? tokenType_default.RIGHT_BRACE : tokenType_default.RIGHT_BRACKET)) {
          this.stack.pop();
          return;
        }
        fail("invalidData");
      }
      if (frame?.state === "key" || frame?.state === "keyOrEnd") {
        if (frame.state === "keyOrEnd" && token === tokenType_default.RIGHT_BRACE) {
          this.stack.pop();
          return;
        }
        if (token !== tokenType_default.STRING) fail("invalidData");
        frame.key = value;
        frame.state = "colon";
        return;
      }
      if (frame?.state === "valueOrEnd" && token === tokenType_default.RIGHT_BRACKET) {
        this.stack.pop();
        return;
      }
      if (!frame && this.rootDone) fail("invalidData");
      const path = frame ? [...frame.path, frame.object ? frame.key : frame.index++] : [];
      const recordStart = frame?.object ? frame.valueStart : offset;
      if (this.previousScalar !== null) this.previousScalar = recordStart;
      if (frame) frame.state = "comma";
      else this.rootDone = true;
      if (token === tokenType_default.LEFT_BRACE || token === tokenType_default.LEFT_BRACKET) {
        if (this.stack.length >= 256) fail("invalidData");
        const object = token === tokenType_default.LEFT_BRACE;
        this.visit(path, object ? {} : []);
        const relative = this.cold && path[0] === "character" ? path.slice(1) : path;
        const streamed = relative.length === 0 || relative.length === 1 && (relative[0] === "chats" || this.cold && relative[0] === "message") || relative[0] === "chats" && typeof relative[1] === "number" && (relative.length === 2 || relative.length === 3 && relative[2] === "message");
        this.stack.push({ path, object, state: object ? "keyOrEnd" : "valueOrEnd", index: 0, start: recordStart, bounded: !streamed });
      } else if ([tokenType_default.STRING, tokenType_default.NUMBER, tokenType_default.TRUE, tokenType_default.FALSE, tokenType_default.NULL].includes(token)) {
        this.visit(path, value);
      } else fail("invalidData");
    }
    write(bytes) {
      this.receivedBytes += bytes.length;
      this.pending += bytes.length;
      if (this.pending > this.maxString + 256 * 1024) fail("recordTooLarge");
      try {
        const text2 = this.decoder.decode(bytes, { stream: true });
        if (text2) this.tokenizer.write(text2);
      } catch (error) {
        if (error instanceof ExportError) throw error;
        fail("invalidData");
      }
    }
    end() {
      try {
        const tail = this.decoder.decode();
        if (tail) this.tokenizer.write(tail);
        this.tokenizer.end();
        this.checkScalarEnd(this.receivedBytes);
      } catch (error) {
        if (error instanceof ExportError) throw error;
        fail("invalidData");
      }
      if (this.stack.length || !this.rootDone && !this.blank) fail("truncated");
    }
    checkScalarEnd(offset) {
      if (this.previousScalar !== null && offset - this.previousScalar > this.maxString) fail("recordTooLarge");
      this.previousScalar = null;
    }
  };
  function references() {
    const assets = /* @__PURE__ */ new Set(), cold = /* @__PURE__ */ new Set(), pluginCold = /* @__PURE__ */ new Set();
    let token = "";
    function addCold(key) {
      if (!UUID.test(key)) fail("invalidCold");
      cold.add(key);
    }
    function visitPluginStorage(path, value) {
      if (typeof value !== "string" || !value) return;
      if (path.length === 2 && path[0] === "_coldplugin" || path.length === 1 && path[0] === "_coldpluginIndex") pluginCold.add(value);
    }
    return {
      assets,
      cold,
      pluginCold,
      visitPluginStorage,
      get token() {
        return token;
      },
      visit(path, value) {
        if (typeof value !== "string") return;
        const key = path.at(-1), parent = path.at(-2);
        if (path[0] === "pluginCustomStorage") visitPluginStorage(path.slice(1), value);
        if (key === "token" && parent === "account") token = value;
        if (key === "coldstorage" && value) addCold(value);
        if (parent === "coldStoragedChats" && value) addCold(value);
        if (key === "data" && value.startsWith(COLD_HEADER)) addCold(value.slice(COLD_HEADER.length));
        if (!value.startsWith("assets/")) return;
        if (["customBackground", "userIcon", "image", "icon", "imgFile"].includes(key) || key === "img" && path.at(-3) === "characterOrder" || key === "uri" && path.includes("ccAssets") || parent === "files" && path.includes("vits") || key === 1 && ["emotionImages", "additionalAssets", "assets"].includes(path.at(-3))) {
          assets.add(value);
        }
      }
    };
  }
  async function scanJson(stream, visit, signal3, max = MAX_BLOCK, options) {
    const parser = new JsonScan(visit, options);
    const breathe = yielding(signal3);
    let length = 0;
    for await (const bytes of chunks(stream, signal3)) {
      length += bytes.length;
      if (length > max) fail("recordTooLarge");
      parser.write(bytes);
      await breathe();
    }
    parser.end();
    return length;
  }
  async function scanMessagePack(reader, visit, signal3) {
    const breathe = yielding(signal3);
    async function value(path, asKey = false) {
      check(signal3);
      if (path.length > 256) fail("invalidData");
      const tag = await reader.number(1);
      let length, map = false, array = false;
      if ((tag & 224) === 160) length = tag & 31;
      else if (tag >= 217 && tag <= 219) length = await reader.number([1, 2, 4][tag - 217]);
      if (length != null) {
        if (!asKey && length > MAX_REFERENCE) {
          for await (const _ of reader.parts(length)) await breathe();
          return;
        }
        const text2 = decoder.decode(await reader.take(length));
        if (!asKey) visit(path, text2);
        return text2;
      }
      if ((tag & 240) === 128) {
        length = tag & 15;
        map = true;
      } else if ((tag & 240) === 144) {
        length = tag & 15;
        array = true;
      } else if (tag === 220 || tag === 221) {
        length = await reader.number(tag === 220 ? 2 : 4);
        array = true;
      } else if (tag === 222 || tag === 223) {
        length = await reader.number(tag === 222 ? 2 : 4);
        map = true;
      }
      if (map || array) {
        if (asKey) fail("invalidData");
        visit(path, map ? {} : []);
        for (let i = 0; i < length; i++) {
          const key = map ? await value(path, true) : i;
          if (map && typeof key !== "string") fail("invalidData");
          await value([...path, key]);
          if (i % 256 === 0) await breathe();
        }
        return;
      }
      if (asKey) fail("invalidData");
      if (tag <= 127 || tag >= 224 || [192, 194, 195].includes(tag)) {
        visit(path, tag === 192 ? null : tag === 194 ? false : tag === 195 ? true : tag <= 127 ? tag : tag - 256);
        return;
      }
      const sizes = { 202: 4, 203: 8, 204: 1, 205: 2, 206: 4, 207: 8, 208: 1, 209: 2, 210: 4, 211: 8 };
      if (sizes[tag]) {
        await reader.take(sizes[tag]);
        return;
      }
      if (tag >= 196 && tag <= 198) length = await reader.number([1, 2, 4][tag - 196]);
      else if (tag >= 199 && tag <= 201) {
        length = await reader.number([1, 2, 4][tag - 199]);
        length++;
      } else if (tag >= 212 && tag <= 216) length = (1 << tag - 212) + 1;
      else fail("unsupported");
      for await (const _ of reader.parts(length)) await breathe();
    }
    await value([]);
    if (await reader.available()) fail("invalidData");
  }
  async function* tap(parts, write) {
    for await (const part of parts) {
      await write(part);
      yield part;
    }
  }
  async function* hold(parts, held) {
    for await (const part of parts) {
      held.push(part.slice());
      yield part;
    }
  }
  var OutputError = class {
    constructor(error) {
      this.error = error;
    }
  };
  async function scanDatabase(stream, refs, signal3, onChunk, maxDecodedBlockBytes = MAX_BLOCK, write = null) {
    const reader = new ByteReader(stream, signal3, onChunk);
    const output = write;
    write = output && (async (bytes) => {
      try {
        await output(bytes);
      } catch (error) {
        throw new OutputError(error);
      }
    });
    try {
      const first = await reader.take(9);
      if (decoder.decode(first) !== "RISUSAVE\0") {
        const tail = await reader.take(2);
        const prefix = [...first, ...tail];
        if (decoder.decode(Uint8Array.from(prefix.slice(0, 10))) !== "\0RISUSAVE\0" || ![7, 8, 9].includes(prefix[10])) fail("unsupported");
        await write?.(Uint8Array.from(prefix));
        const rest = streamOf(write ? tap(reader.rest(), write) : reader.rest());
        const decoded = prefix[10] === 7 ? rest : rest.pipeThrough(new DecompressionStream("gzip"));
        const limited = new ByteReader(decoded, signal3, (bytes) => {
          decodedBytes += bytes.length;
          if (decodedBytes > maxDecodedBlockBytes) fail("recordTooLarge");
        });
        let decodedBytes = 0;
        try {
          await scanMessagePack(limited, refs.visit, signal3);
        } finally {
          await limited.close();
        }
        return;
      }
      await write?.(first);
      const names = /* @__PURE__ */ new Set(), directory = /* @__PURE__ */ new Set();
      let directoryFound = false;
      const expected = /* @__PURE__ */ new Map([[0, "config"], [1, "root"], [4, "preset"], [5, "modules"], [9, "plugins"], [10, "loadouts"], [11, "pluginStorage"]]);
      while (await reader.available()) {
        const head = await reader.take(3);
        const [type, compression, nameLength] = head;
        const label = await reader.take(nameLength);
        const name = decoder.decode(label);
        const size = await reader.take(4);
        const length = new DataView(size.buffer, size.byteOffset, 4).getUint32(0, true);
        if (!name || names.has(name) || compression > 1) fail("invalidData");
        if (![0, 1, 2, 4, 5, 7, 9, 10, 11].includes(type)) fail("unsupported");
        if (expected.has(type) && expected.get(type) !== name) fail("invalidData");
        names.add(name);
        const header = new Uint8Array(7 + nameLength);
        header.set(head);
        header.set(label, 3);
        header.set(size, 3 + nameLength);
        const held = write && type === 11 && length <= MAX_HELD_PLUGIN_STORAGE ? [] : null;
        if (write && !held) await write(header);
        let characterId = null, rootType = null;
        const parts = reader.parts(length);
        let block = streamOf(held ? hold(parts, held) : write ? tap(parts, write) : parts);
        if (compression) block = block.pipeThrough(new DecompressionStream("gzip"));
        await scanJson(block, (path, value) => {
          if (!path.length) rootType = Array.isArray(value) ? "array" : value && typeof value === "object" ? "object" : "scalar";
          if (path.length === 1 && path[0] === "chaId") characterId = value;
          if (type === 1 && path[0] === "__directory") {
            if (path.length === 1) {
              if (!Array.isArray(value)) fail("invalidData");
              directoryFound = true;
            } else if (path.length === 2) {
              if (typeof value !== "string" || !value || value === "root" || directory.has(value)) fail("invalidData");
              directory.add(value);
            } else fail("invalidData");
          }
          if (type === 11) refs.visitPluginStorage?.(path, value);
          else if (![0, 9, 10].includes(type)) refs.visit(path, value);
        }, signal3, maxDecodedBlockBytes, { maxString: type === 2 || type === 7 ? MAX_STRING : maxDecodedBlockBytes, blank: type === 11 });
        if (type === 11 && rootType === null) {
          if (write && !held) fail("invalidData");
          if (held) await write(EMPTY_PLUGIN_STORAGE);
        } else {
          if ([4, 5, 9, 10].includes(type) ? rootType !== "array" : rootType !== "object") fail("invalidData");
          if (held) {
            await write(header);
            for (const part of held) await write(part);
          }
        }
        if (type === 2 || type === 7) {
          if (characterId !== name) fail("invalidData");
        }
      }
      for (const name of expected.values()) if (!names.has(name)) fail("invalidData");
      if (!directoryFound) fail("invalidData");
      for (const name of directory) if (!names.has(name)) fail("invalidData");
      for (const name of names) if (name !== "root" && !directory.has(name)) fail("invalidData");
    } catch (error) {
      if (error instanceof OutputError) throw error.error;
      if (error instanceof ExportError || signal3?.aborted) throw error;
      fail("invalidData");
    } finally {
      await reader.close();
    }
  }

  // src/storage.mjs
  var DB_KEY = "database/database.bin";
  function checkPlatform() {
    if (globalThis.__NODE__) fail("nodeServer");
  }
  var IdbStore = class _IdbStore {
    constructor(db, store) {
      this.db = db;
      this.store = store;
    }
    static async open(name, store = "keyvaluepairs", factory = indexedDB) {
      return new Promise((resolve, reject) => {
        let missing = false, blocked = false;
        const request = factory.open(name);
        request.onupgradeneeded = () => {
          missing = true;
          request.transaction.abort();
        };
        request.onerror = () => missing ? resolve(null) : reject(request.error);
        request.onblocked = () => {
          blocked = true;
          reject(new ExportError("storageBlocked"));
        };
        request.onsuccess = () => {
          const db = request.result;
          if (blocked) {
            db.close();
            return;
          }
          if (!db.objectStoreNames.contains(store)) {
            db.close();
            resolve(null);
          } else {
            db.onversionchange = () => db.close();
            resolve(new _IdbStore(db, store));
          }
        };
      });
    }
    get(key) {
      return new Promise((resolve, reject) => {
        check(this.signal);
        const tx = this.db.transaction(this.store, "readonly");
        const abort = () => {
          try {
            tx.abort();
          } catch {
          }
        };
        const cleanup = () => this.signal?.removeEventListener("abort", abort);
        this.signal?.addEventListener("abort", abort, { once: true });
        const request = tx.objectStore(this.store).get(key);
        request.onsuccess = () => {
          cleanup();
          resolve(request.result);
        };
        request.onerror = () => {
          cleanup();
          reject(this.signal?.aborted ? this.signal.reason : request.error);
        };
        tx.onabort = () => {
          cleanup();
          reject(this.signal?.aborted ? this.signal.reason : tx.error);
        };
      });
    }
    async keys(signal3) {
      const all = [];
      let last;
      do {
        check(signal3);
        const batch = await new Promise((resolve, reject) => {
          const tx = this.db.transaction(this.store, "readonly");
          const request = tx.objectStore(this.store).getAllKeys(last == null ? null : IDBKeyRange.lowerBound(last, true), 512);
          request.onsuccess = () => resolve(request.result);
          request.onerror = () => reject(request.error);
        });
        if (!batch.length) break;
        if (batch.some((key) => typeof key !== "string")) fail("invalidName");
        all.push(...batch);
        last = batch.at(-1);
        if (batch.length < 512) break;
      } while (true);
      return all;
    }
    close() {
      this.db.close();
    }
  };
  var AccountReader = class {
    constructor({ storage = localStorage, fetcher = (...args) => globalThis.fetch(...args), getKeys, signal: signal3, legacyToken = "" }) {
      this.storage = storage;
      this.fetcher = fetcher;
      this.getKeys = getKeys;
      this.signal = signal3;
      this.legacyToken = legacyToken;
      this.accessToken = "";
      this.expires = 0;
      this.refreshing = null;
      this.hub = storage.getItem("hub") === "nightly" ? "https://nightly.sv.risuai.xyz" : "https://sv.risuai.xyz";
    }
    async refresh() {
      if (this.refreshing) return this.refreshing;
      this.refreshing = (async () => {
        check(this.signal);
        try {
          const encoded = this.storage.getItem("oauthData.json");
          if (!encoded) fail("login");
          const data = JSON.parse(decoder.decode(Uint8Array.from(atob(encoded), (c) => c.charCodeAt(0))));
          if (!data.refresh_token || !data.client_id) fail("login");
          const keys = await this.getKeys();
          if (!keys?.privateKey || keys.privateKey.extractable) fail("login");
          const config = await discovery(new URL("https://account.sionyw.com/"), data.client_id, data.client_secret, void 0, {
            [customFetch2]: (url, options) => this.fetcher(url, { ...options, signal: this.signal })
          });
          const tokens = await refreshTokenGrant(config, data.refresh_token, void 0, { DPoP: getDPoPHandle(config, keys) });
          this.accessToken = tokens.access_token;
          this.expires = Date.now() + (tokens.expires_in ?? 300) * 1e3;
          if (tokens.refresh_token && tokens.refresh_token !== data.refresh_token) {
            if (this.storage.getItem("oauthData.json") !== encoded) fail("changed");
            data.refresh_token = tokens.refresh_token;
            const bytes = encoder.encode(JSON.stringify(data));
            this.storage.setItem("oauthData.json", btoa(Array.from(bytes, (b) => String.fromCharCode(b)).join("")));
          }
        } catch (error) {
          check(this.signal);
          if (error?.code === "changed") throw error;
          fail("login");
        }
      })();
      try {
        await this.refreshing;
      } finally {
        this.refreshing = null;
      }
    }
    async request(path, headers2 = {}) {
      const legacy = this.storage.getItem("ignoreRisuAuth") === "true" ? "" : this.legacyToken;
      for (let attempt = 0; attempt < 4; attempt++) {
        check(this.signal);
        if (!legacy && this.expires - Date.now() < 6e4) await this.refresh();
        let response;
        const timeout = new AbortController();
        const timer = setTimeout(() => timeout.abort(), 6e4);
        try {
          response = await this.fetcher(this.hub + path, {
            method: "GET",
            cache: "no-store",
            signal: AbortSignal.any([this.signal, timeout.signal].filter(Boolean)),
            headers: { ...headers2, ...legacy ? { "x-risu-auth": legacy } : { Authorization: "Bearer " + this.accessToken } }
          });
        } catch {
          check(this.signal);
          if (attempt === 3) fail("network");
          await wait(500 * 2 ** attempt, this.signal);
          continue;
        } finally {
          clearTimeout(timer);
        }
        if ([401, 403].includes(response.status) || response.headers.get("WWW-Authenticate")?.includes("invalid_token")) {
          await response.body?.cancel().catch(() => {
          });
          if (legacy || attempt === 3) fail("login");
          this.expires = 0;
          continue;
        }
        if ([408, 429, 500, 502, 503, 504].includes(response.status)) {
          const retry2 = response.headers.get("retry-after");
          const seconds = Number(retry2);
          const delay = retry2 && Number.isFinite(seconds) ? seconds * 1e3 : retry2 && Number.isFinite(Date.parse(retry2)) ? Date.parse(retry2) - Date.now() : 500 * 2 ** attempt;
          await response.body?.cancel().catch(() => {
          });
          if (attempt === 3) fail("network");
          await wait(Math.max(0, Math.min(delay, 3e4)), this.signal);
          continue;
        }
        return response;
      }
      fail("network");
    }
    async read(key) {
      return this.request("/api/account/read/" + keyHex(key) + (key === DB_KEY ? "|" + crypto.randomUUID() : ""), {
        "x-risu-key": key,
        "x-risu-save-date": "0"
      });
    }
  };
  function responseSource(response, signal3, idle = 6e4) {
    if (!response.ok || response.status === 204 || !response.body) {
      void response.body?.cancel().catch(() => {
      });
      fail(response.status === 204 || response.status === 404 || response.ok ? "missing" : "network");
    }
    const raw = response.headers.get("x-body-size") ?? (response.headers.get("content-encoding") ? null : response.headers.get("content-length"));
    const size = raw !== null && /^\d+$/.test(raw) ? Number(raw) : null;
    let used = false;
    return {
      size,
      stream() {
        if (used) fail("invalidData");
        used = true;
        const reader = response.body.getReader();
        let received = 0;
        const stop = (controller, error) => {
          void reader.cancel().catch(() => {
          });
          controller.error(signal3?.aborted ? signal3.reason : error instanceof ExportError ? error : new ExportError("interrupted"));
        };
        return new ReadableStream({
          async pull(controller) {
            let timer, item;
            try {
              item = await Promise.race([reader.read(), new Promise((_, reject) => {
                timer = setTimeout(() => reject(new ExportError("interrupted")), idle);
              })]);
            } catch (error) {
              stop(controller, error);
              return;
            } finally {
              clearTimeout(timer);
            }
            if (item.done) {
              if (size != null && received !== size) stop(controller, new ExportError("interrupted"));
              else controller.close();
              return;
            }
            received += item.value.length;
            if (size != null && received > size) {
              stop(controller, new ExportError("changed"));
              return;
            }
            controller.enqueue(item.value);
          },
          cancel(reason) {
            return reader.cancel(reason);
          }
        }, { highWaterMark: 0 });
      },
      async cancel() {
        if (!used) await response.body.cancel().catch(() => {
        });
      }
    };
  }
  async function fileAt(root, name) {
    if (!root) return null;
    try {
      return await (await root.getFileHandle(name)).getFile();
    } catch (error) {
      if (error.name === "NotFoundError") return null;
      throw error;
    }
  }
  var BrowserSource = class _BrowserSource {
    static async open(signal3, onMode = () => {
    }) {
      checkPlatform();
      const source = new _BrowserSource();
      source.abortController = new AbortController();
      source.onAbort = () => source.abortController.abort(signal3.reason);
      signal3?.addEventListener("abort", source.onAbort, { once: true });
      source.parentSignal = signal3;
      if (signal3?.aborted) source.onAbort();
      signal3 = source.abortController.signal;
      source.signal = signal3;
      source.stores = [];
      source.flags = ["accountst", "opfs_flag!", "hub", "ignoreRisuAuth"].map((key) => localStorage.getItem(key));
      try {
        const open = async (name, store) => {
          const db = await IdbStore.open(name, store);
          if (db) {
            db.signal = signal3;
            source.stores.push(db);
          }
          return db;
        };
        source.local = await open("risuai");
        source.inlays = await open("inlay", "inlay");
        try {
          source.root = await navigator.storage?.getDirectory();
        } catch (error) {
          if (error.name !== "SecurityError" && error.name !== "NotSupportedError") throw error;
          source.opfsError = error;
        }
        if (source.flags[0] === "able") {
          source.mode = "account";
          source.cache = await open("localforage");
          source.dbCache = await open("risuaiAccountCached");
          let legacyToken = "";
          try {
            legacyToken = JSON.parse(localStorage.getItem("fallbackRisuToken") ?? "{}").token ?? "";
          } catch {
          }
          if (!legacyToken && !localStorage.getItem("oauthData.json") && source.dbCache) {
            const cached = await source.dbCache.get(DB_KEY);
            if (cached) {
              const refs = references();
              await scanDatabase(sourceOf(cached).stream(), refs, signal3);
              legacyToken = refs.token;
            }
          }
          source.account = new AccountReader({
            signal: signal3,
            legacyToken,
            getKeys: async () => {
              const db = await open("DPoPDB", "DPoPStore");
              return db?.get("dpop");
            }
          });
        } else {
          source.mode = "indexeddb";
          if (navigator.storage?.getDirectory && globalThis.FileSystemFileHandle?.prototype?.createWritable && source.flags[1] === "able") {
            if (!source.root) throw source.opfsError ?? new ExportError("denied");
            const keys = await source.local?.keys(signal3) ?? [];
            if (!keys.includes(DB_KEY) || await source.local.get("migrated")) source.mode = "opfs";
            else if (!await source.local.get("denied_opfs")) fail("migration");
          }
        }
        onMode(source.mode);
        return source;
      } catch (error) {
        source.close();
        throw error;
      }
    }
    async inventory() {
      check(this.signal);
      const assets = [], cold = [];
      if (this.root) {
        for await (const [name, handle] of this.root.entries()) {
          if (handle.kind !== "file") continue;
          if (/^coldstorage_[0-9a-f-]+\.json$/i.test(name)) cold.push(name.slice(12, -5));
          if (this.mode === "opfs") {
            const key = fromHex(name);
            if (key?.startsWith("assets/")) assets.push(key);
          }
        }
      }
      if (this.mode !== "opfs") {
        const db = this.mode === "account" ? this.cache : this.local;
        assets.push(...(await db?.keys(this.signal) ?? []).filter((key) => key.startsWith("assets/")));
      }
      const inlays = await this.inlays?.keys(this.signal) ?? [];
      return { assets: assets.sort(), cold: cold.sort(), inlays: inlays.sort() };
    }
    async database() {
      if (this.mode === "account") {
        const response = await this.account.read(DB_KEY);
        if (response.status === 303) {
          const match = await response.json();
          const date = await this.dbCache?.get(DB_KEY + "__date");
          const cached = date === "0" || date === 0 ? await this.dbCache?.get(DB_KEY) : null;
          if (match?.match && cached) return sourceOf(cached);
          fail("changed");
        }
        try {
          return responseSource(response, this.signal);
        } catch (error) {
          if (error.code === "missing") fail("missingDatabase");
          throw error;
        }
      }
      const value = this.mode === "opfs" ? await fileAt(this.root, keyHex(DB_KEY)) : await this.local?.get(DB_KEY);
      if (!value) fail("missingDatabase");
      return sourceOf(value);
    }
    async asset(key) {
      check(this.signal);
      const local = this.mode === "opfs" ? await fileAt(this.root, keyHex(key)) : await (this.mode === "account" ? this.cache : this.local)?.get(key);
      if (local != null) return sourceOf(local);
      if (this.mode !== "account") fail("missing");
      return responseSource(await this.account.read(key), this.signal);
    }
    async cold(key) {
      if (this.mode === "account") {
        const response = await this.account.request("/hub/account/coldstorage", { "x-risu-key": key });
        if (response.status === 200) return responseSource(response, this.signal);
        await response.body?.cancel().catch(() => {
        });
        if (![404, 204].includes(response.status)) fail("network");
      }
      const file = await fileAt(this.root, "coldstorage_" + key + ".json");
      if (!file) fail("missing");
      return sourceOf(file);
    }
    // A removed inlay is skipped here and reported as a change by verifyInventory.
    async inlay(key) {
      return this.inlays?.get(key);
    }
    async verifyInventory(expected) {
      const flags = ["accountst", "opfs_flag!", "hub", "ignoreRisuAuth"].map((key) => localStorage.getItem(key));
      if (JSON.stringify(flags) !== JSON.stringify(this.flags)) fail("changed");
      const actual = await this.inventory();
      if (JSON.stringify(actual) !== JSON.stringify(expected)) fail("changed");
    }
    cancel() {
      this.abortController.abort();
    }
    close() {
      this.parentSignal?.removeEventListener("abort", this.onAbort);
      for (const db of this.stores ?? []) db.close();
    }
  };

  // src/export.mjs
  var MAX_COLD = 1024 * 1024 * 1024;
  var BASE64 = /^[A-Za-z0-9+/]*={0,2}$/;
  var RETRIES = 3;
  var UNREADABLE = /* @__PURE__ */ new Set(["invalidData", "truncated", "recordTooLarge", "invalidCold"]);
  var unreadable = (error) => error instanceof ExportError && UNREADABLE.has(error.code);
  async function* gunzip(source, signal3) {
    try {
      yield* chunks(source.stream().pipeThrough(new DecompressionStream("gzip")), signal3);
    } catch (error) {
      if (error instanceof TypeError) fail("invalidCold");
      throw error;
    }
  }
  function assetName(key) {
    if (!key.startsWith("assets/")) fail("invalidName");
    const name = key.slice(7);
    if (name === "database.risudat" || name === "encryption.risudat" || /^inlay_(?:[0-9a-f]{2})+\.risuinlay$/.test(name) || /^coldstorage_[0-9a-f-]{36}\.json$/i.test(name) || /^coldstorage\/[^/]+\.json$/.test(name) || /^(?:inlay|inlay_sidecar|inlay_info|inlay_meta|inlay_thumb)\//.test(name)) fail("invalidName");
    return name;
  }
  function inlaySource(key, record) {
    if (typeof key !== "string" || !key || key.startsWith("assets/")) fail("invalidInlay");
    if (!record || typeof record !== "object" || typeof record.name !== "string" || typeof record.ext !== "string") fail("invalidInlay");
    const type = record.type ?? (record.data instanceof Blob || /^data:image\//i.test(record.data) ? "image" : null);
    if (!["image", "audio", "video", "signature"].includes(type)) fail("invalidInlay");
    let payload, mime;
    if (record.data instanceof Blob) {
      payload = sourceOf(record.data);
      mime = record.data.type;
    } else if (typeof record.data === "string" && type === "signature") {
      payload = sourceOf(encoder.encode(record.data));
      mime = "application/json";
    } else if (typeof record.data === "string") {
      const comma = record.data.indexOf(",");
      const header2 = record.data.slice(0, comma);
      if (comma < 0 || !/^data:[^,;]*;base64$/i.test(header2)) fail("invalidInlay");
      mime = header2.slice(5, -7);
      const length = record.data.length - comma - 1;
      if (length % 4 !== 0 || !length || !BASE64.test(record.data.slice(comma + 1))) fail("invalidInlay");
      const size = length / 4 * 3 - (record.data.endsWith("==") ? 2 : record.data.endsWith("=") ? 1 : 0);
      payload = {
        size,
        stream: () => streamOf((async function* () {
          for (let offset = comma + 1; offset < record.data.length; offset += 256 * 1024) {
            const binary = atob(record.data.slice(offset, offset + 256 * 1024));
            yield Uint8Array.from(binary, (c) => c.charCodeAt(0));
          }
        })())
      };
    } else fail("invalidInlay");
    const fallbackMime = { jpg: "image/jpeg", jpeg: "image/jpeg", png: "image/png", json: "application/json" };
    mime ||= fallbackMime[record.ext] ?? (type === "signature" ? "application/json" : type + "/" + record.ext);
    const { data, type: _, ...extra } = record;
    const metadata = { ...extra, key, kind: "inlay", inlayType: type, mime, size: payload.size };
    for (const dimension of ["width", "height"]) {
      if (metadata[dimension] != null && (!Number.isSafeInteger(metadata[dimension]) || metadata[dimension] < 0)) fail("invalidInlay");
    }
    let header;
    try {
      header = encoder.encode(JSON.stringify(metadata));
    } catch {
      fail("invalidInlay");
    }
    if (header.length > 1024 * 1024) fail("invalidInlay");
    return {
      name: "inlay_" + keyHex(key) + ".risuinlay",
      size: 4 + header.length + payload.size,
      stream: () => streamOf((async function* () {
        yield u32(header.length);
        yield header;
        yield* chunks(payload.stream());
      })())
    };
  }
  async function exportBackup({ source, sink, signal: signal3, progress = () => {
  } }) {
    const skipped = { missingAssets: 0, missingCold: 0, unreadableCold: 0, pluginCold: 0, unsupportedInlays: 0 };
    const state = { stage: "inventory", mode: source.mode, bytes: 0, readBytes: 0, items: 0, total: null, skipped, started: performance.now() };
    let last = 0;
    const report = (patch = {}, force = false) => {
      Object.assign(state, patch);
      if (force || performance.now() - last >= 100) {
        last = performance.now();
        progress({ ...state, skipped: { ...skipped } });
      }
    };
    const writer = new ArchiveWriter(sink, signal3, (n) => {
      state.bytes += n;
      report();
    });
    const breathe = yielding(signal3);
    const retrying = async (first, reopen, write) => {
      let value = first;
      for (let retry2 = 0; ; retry2++) {
        try {
          return await write(value);
        } catch (error) {
          check(signal3);
          if (error?.code !== "interrupted") throw error;
          if (retry2 === RETRIES) fail("network");
          await writer.rollback();
          await wait(500 * 2 ** retry2, signal3);
          value = await reopen();
        }
      }
    };
    const writeCold = async (key, compressed) => {
      const found = references();
      let root = null, chat = false, size = 0;
      const parser = new JsonScan((path, value) => {
        if (!path.length) root = Array.isArray(value) ? "array" : value && typeof value === "object" ? "object" : "scalar";
        if (path.length === 1 && ["character", "message"].includes(path[0])) chat = true;
        found.visit(path, value);
      }, { cold: true });
      await writer.begin("coldstorage_" + key + ".json");
      for await (const bytes of gunzip(compressed, signal3)) {
        size += bytes.length;
        if (size > MAX_COLD) fail("recordTooLarge");
        parser.write(bytes);
        await writer.append(bytes);
        await breathe();
      }
      parser.end();
      if (root !== "array" && !(root === "object" && chat)) fail("invalidCold");
      await writer.end();
      return found;
    };
    let pending = null;
    try {
      report({}, true);
      const inventory = await source.inventory();
      report({ stage: "database" }, true);
      let refs, initialHash;
      await retrying(await source.database(), () => source.database(), async (database) => {
        refs = references();
        const hash = sha256.create();
        let read = 0;
        await writer.begin("database.risudat");
        try {
          await scanDatabase(database.stream(), refs, signal3, async (bytes) => {
            hash.update(bytes);
            read += bytes.length;
            state.readBytes += bytes.length;
            await breathe();
          }, void 0, (bytes) => writer.append(bytes));
        } catch (error) {
          state.readBytes -= read;
          throw error;
        }
        if (database.size != null && read !== database.size) fail("changed");
        await writer.end();
        initialHash = hex(hash.digest());
      });
      state.items++;
      report({ stage: "cold", total: null }, true);
      const coldKeys = new Set(refs.cold);
      for (const key of coldKeys) {
        check(signal3);
        let found;
        try {
          found = await retrying(await source.cold(key), () => source.cold(key), (compressed) => writeCold(key, compressed));
        } catch (error) {
          check(signal3);
          const reason = error?.code === "missing" ? "missingCold" : unreadable(error) ? "unreadableCold" : null;
          if (!reason) throw error;
          await writer.rollback();
          skipped[reason]++;
          report();
          continue;
        }
        for (const asset of found.assets) refs.assets.add(asset);
        for (const next of found.cold) coldKeys.add(next);
        state.items++;
        report();
      }
      for (const key of refs.pluginCold) if (!coldKeys.has(key)) skipped.pluginCold++;
      const assets = [.../* @__PURE__ */ new Set([...inventory.assets, ...refs.assets])].sort();
      report({ stage: "assets", total: state.items + assets.length + inventory.inlays.length }, true);
      const prepare = (key) => source.asset(key).then((value) => ({ value }), (error) => ({ error }));
      if (assets.length) pending = prepare(assets[0]);
      for (let i = 0; i < assets.length; i++) {
        check(signal3);
        const key = assets[i], name = assetName(key);
        const item = await pending;
        pending = null;
        if (item.error && item.error.code !== "missing") throw item.error;
        if (i + 1 < assets.length && source.mode === "account") pending = prepare(assets[i + 1]);
        try {
          if (item.error) throw item.error;
          await retrying(item.value, () => source.asset(key), (value) => writer.entry(name, value));
          state.items++;
        } catch (error) {
          check(signal3);
          if (error?.code !== "missing") throw error;
          await writer.rollback();
          skipped.missingAssets++;
          state.total--;
        }
        report();
        await breathe();
        if (i + 1 < assets.length && !pending) pending = prepare(assets[i + 1]);
      }
      report({ stage: "inlays" }, true);
      for (const key of inventory.inlays) {
        check(signal3);
        try {
          const entry = inlaySource(key, await source.inlay(key));
          await writer.entry(entry.name, entry);
          state.items++;
        } catch (error) {
          check(signal3);
          if (error?.code !== "invalidInlay") throw error;
          await writer.rollback();
          skipped.unsupportedInlays++;
          state.total--;
        }
        report();
        await breathe();
      }
      report({ stage: "verify" }, true);
      await source.verifyInventory(inventory);
      const finalHash = await retrying(
        await source.database(),
        () => source.database(),
        (database) => digest(database, signal3, (n) => {
          state.readBytes += n;
          report();
        })
      );
      if (initialHash !== finalHash) fail("changed");
      await source.verifyInventory(inventory);
      report({ stage: "finishing" }, true);
      await writer.finish();
      report({ stage: "complete" }, true);
      return { bytes: writer.position, items: state.items, skipped: { ...skipped } };
    } catch (error) {
      source.cancel?.();
      await writer.abort();
      throw error;
    } finally {
      if (pending) {
        const next = await pending;
        await next.value?.cancel?.();
      }
      source.close?.();
    }
  }

  // src/ui.mjs
  var text = {
    en: {
      title: "RisuNest backup export",
      intro: "Export this browser’s saved RisuAI data, assets, and all local inlays.",
      idle: "Ready",
      confirmed: "Saving has finished. I have stopped generation and edits in all tabs and devices.",
      start: "Export backup",
      cancel: "Cancel",
      close: "Close",
      language: "Language",
      inventory: "Listing saved data",
      database: "Reading database",
      cold: "Exporting archived chats",
      assets: "Exporting assets",
      inlays: "Exporting inlays",
      verify: "Checking for changes",
      finishing: "Finishing file",
      complete: "Backup saved",
      prepared: "Backup file ready",
      cancelled: "Export cancelled",
      written: "Written",
      items: "Items",
      elapsed: "Elapsed",
      speed: "Write speed",
      read: "Database read",
      skipped: "Skipped",
      skippedNote: "The following items were skipped. Everything else is included in the backup.",
      missingAssets: "Missing assets",
      missingCold: "Missing archived chats",
      unreadableCold: "Unreadable archived chats",
      pluginCold: "Plugin storage data",
      unsupportedInlays: "Unsupported inlays",
      local: "Local",
      indexeddb: "Local · IndexedDB",
      opfs: "Local · OPFS",
      account: "Account",
      download: "Download backup",
      downloadStarted: "Download started. Check your browser’s downloads before removing the temporary file.",
      firefox: "This browser uses a temporary file before downloading. Additional storage roughly equal to the backup size is required. Keep this tab open until the download finishes.",
      cleanup: "Remove temporary files",
      cleaned: "Temporary files removed",
      cleanupConfirm: "Remove export temporary files? Wait for any backup download to finish first.",
      active: "An export is already running.",
      failed: "Export failed. No complete backup was saved.",
      changed: "Saved data changed during export. Stop edits and generation, wait for saving to finish, then try again.",
      missingDatabase: "No saved RisuAI database was found on this page.",
      nodeServer: "This RisuAI page saves data on its server, not in this browser. Back up the server’s save folder instead.",
      login: "Account authentication failed. Sign in again in RisuAI, wait for saving to finish, then retry.",
      network: "An account download failed. Check the connection and try again.",
      migration: "A storage migration is incomplete. Reload RisuAI and wait for it to finish before exporting.",
      unsupported: "This saved database format is not supported. Open and save it in the supported RisuAI release, then retry.",
      invalidData: "Saved data is malformed or unsupported. Export was stopped.",
      invalidCold: "An archived-chat reference is invalid. Export was stopped.",
      invalidName: "An asset name cannot be preserved in this backup format. Export was stopped.",
      duplicate: "Two entries would use the same backup name. Export was stopped.",
      truncated: "A saved file ended unexpectedly. Export was stopped.",
      recordTooLarge: "A database block or record exceeds the supported import limits.",
      entryTooLarge: "A single backup entry exceeds 4 GiB minus 1 byte.",
      quota: "Not enough browser storage for the temporary backup. Free space or use Chrome or Edge.",
      denied: "File or storage access was denied. Check browser permissions and retry.",
      outputUnavailable: "This browser does not provide a supported file output method.",
      storageBlocked: "Browser storage is busy. Close other RisuAI tabs and retry.",
      persist: "Request storage permission",
      permissionPending: "Respond to the browser’s storage permission request.",
      permissionGranted: "Storage permission granted. Try exporting again.",
      permissionDenied: "Storage permission was not granted."
    },
    ko: {
      title: "RisuNest 백업 내보내기",
      intro: "이 브라우저에 저장된 웹리스 데이터와 에셋, 모든 로컬 인레이를 내보냅니다.",
      idle: "준비",
      confirmed: "저장이 완료되었으며, 모든 탭과 기기에서 생성과 편집을 중지했습니다.",
      start: "백업 내보내기",
      cancel: "취소",
      close: "닫기",
      language: "언어",
      inventory: "저장된 데이터 확인 중",
      database: "DB 읽는 중",
      cold: "보관된 채팅 내보내는 중",
      assets: "에셋 내보내는 중",
      inlays: "인레이 내보내는 중",
      verify: "변경 사항 확인 중",
      finishing: "파일 저장 마무리 중",
      complete: "백업 저장 완료",
      prepared: "백업 파일 준비 완료",
      cancelled: "내보내기가 취소되었습니다",
      written: "기록한 용량",
      items: "항목",
      elapsed: "경과 시간",
      speed: "기록 속도",
      read: "읽은 DB 용량",
      skipped: "제외한 항목",
      skippedNote: "다음 항목은 백업에서 제외했습니다. 나머지 데이터는 모두 백업에 포함되었습니다.",
      missingAssets: "찾을 수 없는 에셋",
      missingCold: "찾을 수 없는 보관된 채팅",
      unreadableCold: "읽을 수 없는 보관된 채팅",
      pluginCold: "플러그인 저장 데이터",
      unsupportedInlays: "지원하지 않는 인레이",
      local: "로컬",
      indexeddb: "로컬 · IndexedDB",
      opfs: "로컬 · OPFS",
      account: "계정",
      download: "백업 다운로드",
      downloadStarted: "다운로드를 시작했습니다. 브라우저에서 다운로드 완료를 확인한 후 임시 파일을 삭제하세요.",
      firefox: "이 브라우저는 임시 파일을 만든 후 다운로드합니다. 백업 크기만큼의 추가 저장 공간이 필요합니다. 다운로드가 끝날 때까지 이 탭을 열어두세요.",
      cleanup: "임시 파일 삭제",
      cleaned: "임시 파일을 삭제했습니다",
      cleanupConfirm: "내보내기 임시 파일을 삭제하시겠습니까? 백업 다운로드가 끝난 후 삭제해주세요.",
      active: "내보내기가 진행 중입니다.",
      failed: "내보내기에 실패했습니다. 완성된 백업이 저장되지 않았습니다.",
      changed: "내보내기 중 저장된 데이터가 변경되었습니다. 편집과 생성을 중지하고 저장이 완료된 후 다시 시도해주세요.",
      missingDatabase: "이 페이지에서 저장된 웹리스 DB를 찾을 수 없습니다.",
      nodeServer: "이 웹리스 페이지는 데이터를 브라우저가 아닌 서버에 저장합니다. 서버의 저장 폴더를 대신 백업해주세요.",
      login: "계정 인증에 실패했습니다. 웹리스에 다시 로그인하고 저장이 완료된 후 다시 시도해주세요.",
      network: "계정 데이터 다운로드에 실패했습니다. 연결을 확인하고 다시 시도해주세요.",
      migration: "저장소 이동이 완료되지 않았습니다. 웹리스를 새로고침하고 이동이 끝난 후 내보내기해주세요.",
      unsupported: "지원하지 않는 DB 저장 형식입니다. 지원하는 웹리스 릴리즈에서 열고 저장한 후 다시 시도해주세요.",
      invalidData: "저장된 데이터가 잘못되었거나 지원하지 않는 형식입니다. 내보내기가 중지되었습니다.",
      invalidCold: "보관된 채팅 참조가 잘못되었습니다. 내보내기가 중지되었습니다.",
      invalidName: "백업 형식에서 보존할 수 없는 에셋 이름입니다. 내보내기가 중지되었습니다.",
      duplicate: "백업 파일 이름이 중복됩니다. 내보내기가 중지되었습니다.",
      truncated: "저장된 파일이 중간에 끊겼습니다. 내보내기가 중지되었습니다.",
      recordTooLarge: "DB 블록 또는 레코드가 지원하는 복원 크기를 초과합니다.",
      entryTooLarge: "개별 백업 항목이 4 GiB에서 1바이트를 뺀 크기를 초과합니다.",
      quota: "임시 백업을 만들 브라우저 저장 공간이 부족합니다. 공간을 확보하거나 Chrome 또는 Edge를 사용해주세요.",
      denied: "파일 또는 저장소 접근이 거부되었습니다. 브라우저 권한을 확인하고 다시 시도해주세요.",
      outputUnavailable: "이 브라우저에서 지원하는 파일 저장 방법이 없습니다.",
      storageBlocked: "브라우저 저장소를 사용할 수 없습니다. 다른 웹리스 탭을 닫고 다시 시도해주세요.",
      persist: "저장 권한 요청",
      permissionPending: "브라우저의 저장 권한 요청을 확인해주세요.",
      permissionGranted: "저장 권한이 허용되었습니다. 내보내기를 다시 시도해주세요.",
      permissionDenied: "저장 권한이 허용되지 않았습니다."
    }
  };
  var TEMP_DIR = "risunest-web-export-v1";
  var TEMP_NAME = /^archive-[0-9a-f-]{36}\.bin$/;
  var formatBytes = (value) => {
    if (value < 1024) return value + " B";
    const unit = Math.min(3, Math.floor(Math.log(value) / Math.log(1024)));
    return (value / 1024 ** unit).toFixed(1) + " " + ["B", "KiB", "MiB", "GiB"][unit];
  };
  async function destination(picker, signal3) {
    if (picker) {
      const handle2 = await picker;
      check(signal3);
      return { sink: await handle2.createWritable(), temporary: false };
    }
    if (!navigator.storage?.getDirectory) fail("outputUnavailable");
    check(signal3);
    const directory = await (await navigator.storage.getDirectory()).getDirectoryHandle(TEMP_DIR, { create: true });
    const name = "archive-" + crypto.randomUUID() + ".bin";
    const handle = await directory.getFileHandle(name, { create: true });
    try {
      const sink = await handle.createWritable();
      return { sink, temporary: true, file: () => handle.getFile(), remove: () => directory.removeEntry(name) };
    } catch (error) {
      await directory.removeEntry(name).catch(() => {
      });
      throw error;
    }
  }
  async function clearTemporaryFiles() {
    if (!navigator.storage?.getDirectory) return;
    let dir;
    try {
      dir = await (await navigator.storage.getDirectory()).getDirectoryHandle(TEMP_DIR);
    } catch (error) {
      if (error.name === "NotFoundError") return;
      throw error;
    }
    for await (const [name, handle] of dir.entries()) {
      if (handle.kind === "file" && TEMP_NAME.test(name)) await dir.removeEntry(name);
    }
    try {
      await (await navigator.storage.getDirectory()).removeEntry(TEMP_DIR);
    } catch (error) {
      if (!["NotFoundError", "InvalidModificationError"].includes(error.name)) throw error;
    }
  }
  function mount() {
    const existing = document.getElementById("risunest-web-export");
    if (existing) {
      existing.scrollIntoView();
      return;
    }
    const host = document.createElement("div");
    host.id = "risunest-web-export";
    const root = host.attachShadow({ mode: "open" });
    root.innerHTML = `<style>
:host{all:initial;position:fixed;right:20px;bottom:20px;width:min(430px,calc(100vw - 24px));z-index:2147483647;color-scheme:dark;font:14px/1.5 system-ui,sans-serif;color:#eef2f6}
*{box-sizing:border-box}section{background:#18202b;border:1px solid #445163;border-radius:14px;padding:20px;box-shadow:0 12px 48px #0008;max-height:calc(100vh - 40px);overflow:auto}
header{display:flex;align-items:center;gap:10px}h1{font-size:18px;margin:0;flex:1}p{margin:12px 0;color:#c1cbd7}select,button{font:inherit;color:inherit;background:#283547;border:1px solid #53647b;border-radius:7px;padding:7px 10px}
button{cursor:pointer}button:disabled{opacity:.5;cursor:default}button.primary{background:#326ca4;border-color:#7daedb}button:hover:enabled{filter:brightness(1.15)}label{display:flex;gap:8px;margin:14px 0}input{margin-top:5px;flex:none}progress{width:100%;height:12px;accent-color:#7cb8ee}
dl{display:grid;grid-template-columns:1fr 1fr;gap:5px 12px;margin:12px 0}dt{color:#aebed0}dd{margin:0;text-align:right;font-variant-numeric:tabular-nums}.actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:16px}.note{font-size:12px}.status{font-weight:600;color:#e4f0ff}.error{color:#ffb6b6}[hidden]{display:none!important}select{max-width:100px}
@media(max-width:480px){:host{right:12px;bottom:12px}section{padding:16px}}
</style><section role="dialog" aria-labelledby="export-title">
<header><h1 id="export-title"></h1><select aria-label="Language"><option value="ko">한국어</option><option value="en">English</option></select></header>
<p data-text="intro"></p><p class="note" data-text="firefox" hidden></p>
<label><input type="checkbox"><span data-text="confirmed"></span></label>
<p class="status" role="status" aria-live="polite"></p><progress hidden></progress>
<dl hidden><dt data-text="written"></dt><dd data-value="bytes"></dd><dt data-text="items"></dt><dd data-value="items"></dd><dt data-text="elapsed"></dt><dd data-value="elapsed"></dd><dt data-text="speed"></dt><dd data-value="speed"></dd><dt data-text="read"></dt><dd data-value="read"></dd><dt data-text="skipped" data-skipped hidden></dt><dd data-value="skipped" data-skipped hidden></dd></dl>
<div class="skipped" hidden><p class="note" data-text="skippedNote"></p><dl></dl></div>
<div class="actions"><button class="primary" data-action="start" data-text="start" disabled></button><button data-action="cancel" data-text="cancel" hidden></button><button class="primary" data-action="download" data-text="download" hidden></button><button data-action="persist" data-text="persist" hidden></button><button data-action="cleanup" data-text="cleanup" hidden></button><button data-action="close" data-text="close"></button></div>
</section>`;
    document.documentElement.append(host);
    const find = (selector) => root.querySelector(selector);
    const button = (action) => find('[data-action="' + action + '"]');
    let lang = navigator.language.toLowerCase().startsWith("ko") ? "ko" : "en";
    let busy = false, controller, state, status = "idle", output, url, timer, endTime;
    const temporary = typeof window.showSaveFilePicker !== "function";
    find("select").value = lang;
    find('[data-text="firefox"]').hidden = !temporary;
    button("cleanup").hidden = !temporary;
    const t = (key) => text[lang][key] ?? text[lang].failed;
    function render() {
      host.lang = lang;
      find("h1").textContent = t("title");
      find("select").setAttribute("aria-label", t("language"));
      for (const node of root.querySelectorAll("[data-text]")) node.textContent = t(node.dataset.text);
      const statusNode = find(".status");
      statusNode.textContent = t(status) + (busy && state?.mode ? " · " + t(state.mode) : "");
      if (state) {
        const seconds = Math.max(1e-3, ((endTime ?? performance.now()) - state.started) / 1e3);
        find("dl").hidden = false;
        find('[data-value="bytes"]').textContent = formatBytes(state.bytes);
        find('[data-value="read"]').textContent = formatBytes(state.readBytes);
        find('[data-value="items"]').textContent = state.items + (state.total == null ? "" : " / " + state.total);
        find('[data-value="elapsed"]').textContent = Math.floor(seconds / 60) + ":" + String(Math.floor(seconds % 60)).padStart(2, "0");
        find('[data-value="speed"]').textContent = formatBytes(state.bytes / seconds) + "/s";
        const skipped = Object.entries(state.skipped ?? {}).filter(([, count]) => count);
        const total = skipped.reduce((sum, [, count]) => sum + count, 0);
        for (const node of root.querySelectorAll("[data-skipped]")) node.hidden = !total;
        find('[data-value="skipped"]').textContent = total;
        find(".skipped").hidden = !total || busy || state.stage !== "complete";
        find(".skipped dl").replaceChildren(...skipped.flatMap(([reason, count]) => {
          const term = document.createElement("dt"), value = document.createElement("dd");
          term.textContent = t(reason);
          value.textContent = count;
          return [term, value];
        }));
        const progress = find("progress");
        progress.hidden = !busy;
        if (state.total != null && !["verify", "finishing"].includes(state.stage)) {
          progress.max = state.total || 1;
          progress.value = state.items;
        } else progress.removeAttribute("value");
      }
    }
    function errorStatus(error) {
      if (controller?.signal.aborted || error?.name === "AbortError") return "cancelled";
      if (error instanceof ExportError) return error.code;
      if (error?.name === "QuotaExceededError") return "quota";
      if (["NotAllowedError", "SecurityError"].includes(error?.name)) return "denied";
      return "failed";
    }
    function controls() {
      button("start").disabled = busy || !find("input").checked || Boolean(url);
      button("cancel").hidden = !busy;
      button("close").disabled = busy || Boolean(url);
      button("cleanup").disabled = busy;
      button("persist").disabled = busy;
      find("input").disabled = busy;
    }
    find("input").onchange = controls;
    find("select").onchange = (event) => {
      lang = event.target.value;
      render();
    };
    button("cancel").onclick = () => {
      controller?.abort();
      button("cancel").disabled = true;
    };
    button("close").onclick = () => {
      if (!busy) {
        clearInterval(timer);
        host.remove();
      }
    };
    button("download").onclick = () => {
      if (!url) return;
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = "RisuNest-web-backup-" + (/* @__PURE__ */ new Date()).toISOString().slice(0, 10) + ".bin";
      anchor.click();
      status = "downloadStarted";
      render();
    };
    button("cleanup").onclick = async () => {
      if (busy || !window.confirm(t("cleanupConfirm"))) return;
      try {
        if (url) {
          URL.revokeObjectURL(url);
          url = null;
        }
        await clearTemporaryFiles();
        button("download").hidden = true;
        status = "cleaned";
        controls();
        render();
      } catch (error) {
        status = errorStatus(error);
        render();
      }
    };
    button("persist").onclick = async () => {
      status = "permissionPending";
      render();
      button("persist").disabled = true;
      try {
        status = await navigator.storage.persist() ? "permissionGranted" : "permissionDenied";
      } catch {
        status = "permissionDenied";
      }
      button("persist").disabled = false;
      render();
    };
    button("start").onclick = async () => {
      if (busy || !find("input").checked) return;
      busy = true;
      controller = new AbortController();
      state = null;
      endTime = null;
      output = null;
      find("dl").hidden = true;
      find(".skipped").hidden = true;
      find("progress").hidden = true;
      button("cancel").disabled = false;
      find(".status").classList.remove("error");
      status = "inventory";
      controls();
      render();
      let picker;
      try {
        checkPlatform();
        picker = temporary ? null : window.showSaveFilePicker({
          suggestedName: "RisuNest-web-backup-" + (/* @__PURE__ */ new Date()).toISOString().slice(0, 10) + ".bin",
          types: [{ description: "RisuNest backup", accept: { "application/octet-stream": [".bin"] } }]
        });
        output = await destination(picker, controller.signal);
        check(controller.signal);
        timer = setInterval(render, 250);
        const source = await BrowserSource.open(controller.signal);
        await exportBackup({
          source,
          sink: output.sink,
          signal: controller.signal,
          progress(next) {
            state = next;
            status = next.stage;
            render();
          }
        });
        if (output.temporary) {
          url = URL.createObjectURL(await output.file());
          button("download").hidden = false;
          status = "prepared";
        } else status = "complete";
      } catch (error) {
        status = errorStatus(error);
        if (status === "quota" && navigator.storage?.persist) button("persist").hidden = false;
        find(".status").classList.toggle("error", status !== "cancelled");
        await output?.sink.abort().catch(() => {
        });
        await output?.remove?.().catch(() => {
        });
      } finally {
        busy = false;
        endTime = performance.now();
        clearInterval(timer);
        controls();
        render();
      }
    };
    render();
  }

  // src/main.mjs
  mount();
})();
