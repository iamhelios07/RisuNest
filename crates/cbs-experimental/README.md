# Experimental CBS library

This unpublished Rust library evaluates a restricted CBS domain using UTF-16 code units. It is consumed by independent verification executables. Product application crates and frontend entry points do not depend on it.

Run these commands from the repository root. Set `CARGO_TARGET_DIR` to the shared Cargo cache, `CARGO_BUILD_JOBS=1` and `RUST_TEST_THREADS=1` before building.

~~~text
cargo test --locked --manifest-path crates/cbs-experimental/Cargo.toml
cargo build --release --locked --manifest-path benchmarks/rust-cbs/native/Cargo.toml
~~~

The first command runs the library's cancellation and finite-choice tests. The second creates `rust-cbs-resident` (`rust-cbs-resident.exe` on Windows) under the shared target's `release` directory. The [CBS harness](../../benchmarks/rust-cbs/README.md) compares this executable with the actual product parser using synthetic inputs.

## API

`cbs::compile` returns a source-preserving tree. `cbs::evaluate` consumes that tree, a mutable `cbs::Context` and a `cbs::Budget`, returning UTF-16 units or a string error. `cbs::fold_constants` folds supported literal calls, and `cbs::retained_bytes` accounts for program vector capacity. These interfaces are experimental and do not promise stable serialization or ABI compatibility.

The caller owns invocation-local temporary state and must clear it for each new parser invocation. Generated CBS is ordinary output until the caller reaches an explicit parser checkpoint. Compilation alone does not establish runtime eligibility. Unknown or unsupported operations require whole-invocation fallback before any externally observed effect.

`relaxed::prepare` explores every bounded finite-choice outcome against an immutable snapshot before consuming entropy. `relaxed::sample` selects a fresh output leaf using OS entropy or an explicitly supplied verification tape. Duplicate candidates retain their weights. The raw evaluator's seeded random source is for differential verification; it is not a product RNG implementation.

## Limits and lifecycle

Generic compilation accepts at most 2,097,152 source units and depth 128. The default budget permits 1,000,000 node ticks and 4,194,304 output units. Cancellation is checked between nodes; individual string or JSON operations are not interrupted. Output limits and vector-capacity accounting are not hard process-memory caps.

Finite-choice preparation limits source length to 65,536 units, each choice to 16 alternatives, depth to 12 choices, exploration to 512 attempts and 256 leaves, and each output to 262,144 units. It shares a 1,000,000-tick evaluation budget and accounts for at most 2 MiB of tree capacity. Preparation currently creates its own budget and does not accept an external cancellation token.

The library does not manage process transport, resident handles, context revisions, stale results, reactive read dependencies or automatic recovery. The JSONL harness owns its additional context and handle limits. Failure after a sampled choice must not restart with fresh randomness: a host needs preserved logical choices or completed checkpoints. Abort-only handling does not implement equivalent product recovery. Persistent writes and function calls remain unsupported.
