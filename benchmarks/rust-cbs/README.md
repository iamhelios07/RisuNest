# Rust CBS resident experiment

This standalone benchmark consumes the unmodified product parser as its oracle and the [experimental CBS library](../../crates/cbs-experimental/README.md) through a Cargo path dependency. It uses synthetic fixtures and is excluded from product entry points. The executable owns JSONL transport, context snapshots, resident handles and timing controls; the library owns evaluation.

Prepare the checkout with its frozen pnpm dependencies and Playwright Chromium installed. The URL, native executable and output file are explicit inputs; the driver does not discover external checkouts or profiles. Set CARGO_TARGET_DIR to the existing shared cache, CARGO_BUILD_JOBS=1 and RUST_TEST_THREADS=1. Run these commands from the repository root:

~~~text
cargo test --locked --manifest-path crates/cbs-experimental/Cargo.toml
cargo test --locked --manifest-path benchmarks/rust-cbs/native/Cargo.toml
cargo build --release --locked --manifest-path benchmarks/rust-cbs/native/Cargo.toml
pnpm exec vite --config benchmarks/streaming/vite.config.ts --mode agent --host 127.0.0.1 --port <port> --strictPort
node benchmarks/rust-cbs/run.mjs http://127.0.0.1:<port> <shared-target>/release/rust-cbs-resident[.exe] <results.json>
node benchmarks/rust-cbs/run.mjs http://127.0.0.1:<port> <shared-target>/release/rust-cbs-resident[.exe] <timings.json> measure
~~~

Both Rust test targets are also included in `node scripts/testNative.mjs --harness`. Default protocol and host test runs exclude them.

The release build writes rust-cbs-resident (rust-cbs-resident.exe on Windows) under the shared target release directory. The driver writes verification results or measurements to the explicit JSON output path. Product application crates and frontend entry points do not depend on this library or executable.

The driver opens only the synthetic streaming harness, blocks non-loopback requests and Realm paths, and calls the actual product risuChatParser in Chromium. The original parser defines the expected output and number of random draws. Timings require a quiet machine. Browser control round trips are reported separately from browser execution time; pipe timings are not Tauri IPC or rendering measurements.

The native module stores text as UTF-16 code units, including lone surrogates. Compiled nodes retain expression text and nested evaluation order. Dispatch splits the evaluated expression using the original first-colon rule, allowing nested results to change names and argument separators. Unsupported or malformed structural forms return a fallback signal.

The optional fold variant precomputes only literal calls from an explicit pure operation set. The driver compares raw and folded results with the same product oracle, and reports preparation, warm execution, retained capacity accounting and late-fallback costs separately.

The experiment covers selected variable snapshots, temporary variables, arithmetic, comparisons, string operations, arrays, nested JSON stringification, random choices and conditional blocks. Numeric output uses ryu-js. JSON strings retain UTF-16 escapes and integer object keys follow JavaScript enumeration order. Array joining follows JavaScript nested-array and object string conversion. Temporary values reset per evaluate call; a false #when still evaluates its body.

This is a restricted experiment, not a full CBS implementation. Legacy syntax, recursion deeper than 128, unknown operations, functions, loops, persistent writes, large radix values and some arithmetic forms fall back. Lazy bodies require statically known supported calls; unsupported or dynamically named calls reject the entire program even when their branch would be false. A successful compilation alone does not establish eligibility. Runtime context can change dispatch and cause fallback.

The line-oriented native protocol accepts register, context, eval/evaluate, stats and release operations. Program handles are bounded to 64 entries and 8 MiB of accounted Vec payload/capacity. This is an accounting limit, not a measured process-heap cap. Contexts are limited to 4,096 keys and 1,048,576 UTF-16 units. Registering one program preserves the others; replacing context increments its revision. Evaluations can require an expected revision. Release clears resident state. Output uses JSON strings for well-formed UTF-16 and integer arrays for unpaired output units.

The evaluator checks a cancellation token between nodes and after evaluation, and enforces a node budget and output length. It does not interrupt a single long string or JSON operation. The synchronous pipe protocol cannot receive cancellation while processing a request. Unit tests exercise an external cancellation token, including cancellation after evaluator progress. A production host would need an independently reachable cancellation channel and a stale-result check at completion.

The deterministic random source exists only for differential verification. It is not the product random source. Runtime fallback discards private state in this harness; replay after externally observed random draws or host writes is not implemented. Do not enable random or host-effect forms in a product native route based on this experiment.


For a syntax-only protocol probe, send one JSON object per line:

~~~json
{"op":"register","id":"probe","text":"{{length::abc}}","fold":false}
{"op":"eval","id":"probe","context":{"variables":[],"globals":[],"index":0,"user":[],"seed":0}}
{"op":"release"}
~~~

Registration returns program count, retained capacity accounting and fold count, or a fallback reason. Reuse an identifier to replace its program. The eval response also contains output, random draw count and state. A syntax-only probe must leave fold false; folding evaluates literal operations. Empty-context evaluation measures only that context and does not prove admission with actual values or equality to the original parser. Text can be an array of UTF-16 integers when lone surrogate units must be preserved.

## Optional finite RNG experiment

The separate relax-register and relax-eval operations accept a different random sequence while preserving finite candidate sets, duplicate-option weights, logical choice sharing and fresh choices on each invocation. Existing register/eval behavior stays unchanged. Run the synthetic contract checks and timings with:

~~~text
node benchmarks/rust-cbs/relaxed-run.mjs http://127.0.0.1:<port> <shared-target>/release/rust-cbs-resident[.exe] <results.json>
node benchmarks/rust-cbs/relaxed-run.mjs http://127.0.0.1:<port> <shared-target>/release/rust-cbs-resident[.exe] <timings.json> measure
node benchmarks/rust-cbs/relaxed-run.mjs http://127.0.0.1:<port> <shared-target>/release/rust-cbs-resident[.exe] <cache-timings.json> cache-measure
~~~

~~~json
{"op":"relax-register","id":"choice","text":"{{random::A::A::B}}","context":{"variables":[],"globals":[],"index":0,"user":[],"seed":0}}
{"op":"relax-eval","id":"choice"}
{"op":"relax-eval","id":"choice","tape":[0.5]}
~~~

Registration explores every finite outcome using private snapshot and temporary state before consuming entropy. It returns nodes, leaves, retainedBytes, revision and entropyWords:0, or a fallback reason. Any reachable unsupported callback, persistent write, semantic guard or resource bound rejects the entire registration. A failed replacement removes its previous relaxed handle. Context replacement and release invalidate all relaxed handles.

Accepted handles contain output leaves and choice branches. Evaluation without a tape uses operating-system entropy and integer rejection sampling; a supplied unit-interval tape is an explicit verification control. The older seeded differential source is separate. No reproducibility option or product setting is added. Candidate and joint-probability structure are checked by finite outcome enumeration. This does not claim bit-level distribution identity with an engine's unspecified finite-precision Math.random implementation or treat a small empirical sample as a statistical proof.

Supported random forms are finite random choices, bounded ordered safe-integer randint, and bounded integer dice/roll domains. Duplicate options retain separate weighted branches. roll with no arguments returns the product's constant 1. Bare continuous random, fractional/reversed/nonfinite numeric domains, tokenize-accurate mode, deterministic pick/rollp/rollpick/hash and host writes are excluded. These exclusions do not redefine those product operations.

Limits are 16 alternatives per choice, 12 choice levels, 256 leaves, 512 tree nodes, a 65,536-unit source, 262,144 output units per leaf, 2 MiB accounted capacity per tree and a shared 1,000,000-node preflight work budget. At most 64 relaxed handles retain 8 MiB of accounted tree capacity. Independent choices grow exponentially; nine binary choices exceed the current bound. Capacity accounting is not a measured process-heap limit.

Sampling errors return abort rather than fallback. A caller must also abort on pipe timeout, process loss, cancellation or a later failure after sampling; it must not retry the original invocation with fresh choices. For subsequent parser checkpoints, preserve the completed intermediate text and fall back only on the current checkpoint input. Whole-request recovery requires saved logical choices or completed checkpoints. The harness demonstrates these rules and a simulated result-dependent downstream failure; it does not implement asynchronous host recovery or Tauri cancellation.

The cache-measure mode compares the shared-one-choice workload with an equivalent JavaScript cache: the original parser computes both leaves for one immutable context, then each invocation samples a fresh leaf. It aggregates browser invocations to reduce timer quantization, checks the UTF-16 unit that differs between choices against each selected index, verifies checksums and draw counts, and reports preparation, context invalidation and stored string payload separately. This distinguishes precomputation from a language or transport advantage. JavaScript string payload bytes and native Vec capacity accounting are different measures; neither reports actual heap usage.

Abort-only handling prevents fresh-draw retry bias in this bounded experiment. It does not reproduce an existing product's automatic recovery behavior and is not permission to change that behavior. Product adoption requires equivalent logical-choice replay or a recovery boundary that preserves both completed choices and non-RNG behavior. Selective reactive read-dependency registration is also outside this snapshot-only protocol.
