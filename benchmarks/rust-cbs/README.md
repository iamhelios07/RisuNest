# Rust CBS resident experiment

This standalone benchmark consumes the unmodified product parser as its oracle. It uses synthetic fixtures and is excluded from product entry points.

Prepare the checkout with its frozen pnpm dependencies and Playwright Chromium installed. The URL, native executable and output file are explicit inputs; the driver does not discover external checkouts or profiles. Set CARGO_TARGET_DIR to the existing shared cache and CARGO_BUILD_JOBS=1. Build and run:

~~~text
cargo test --locked --manifest-path benchmarks/rust-cbs/native/Cargo.toml
cargo build --release --locked --manifest-path benchmarks/rust-cbs/native/Cargo.toml
pnpm exec vite --config benchmarks/streaming/vite.config.ts --mode agent --host 127.0.0.1 --port <port> --strictPort
node benchmarks/rust-cbs/run.mjs http://127.0.0.1:<port> <shared-target>/release/rust-cbs-resident[.exe] <results.json>
node benchmarks/rust-cbs/run.mjs http://127.0.0.1:<port> <shared-target>/release/rust-cbs-resident[.exe] <timings.json> measure
~~~

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
