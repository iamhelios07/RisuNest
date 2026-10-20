# Synthetic send and preparation benchmark

This harness calls the real `sendChat`, parser, scripts, templates, history admission, tokenizer, request wrapper, response application, persistence runtime, and IndexedDB adapter. Its provider is an in-process synthetic entry in the existing provider registry. It makes no provider or other network requests. `fake-indexeddb` supplies the storage I/O boundary. The shared test network guard blocks unexpected requests and RisuRealm.

Run the correctness smoke through the harness test project:

```sh
pnpm exec vitest run --project harness benchmarks/generation-send/send.test.ts --maxWorkers 1 --no-file-parallelism
```

Use the dedicated configuration to include timing probes in a single case:

```sh
pnpm exec vitest run --config benchmarks/generation-send/vitest.config.ts
```

Set `GENERATION_SEND_PROBES=coarse` to match the original coarse baseline instrumentation: history-window preparation, current-chat parsing, output listeners, encoding, complete promotion and IndexedDB commit only. This mode disables fine AST ranges, the forwarding `tokenizeChat` observer and zone/character counters. The default `fine` mode adds the detailed probes described below. Every result records its probe mode.

The single-case environment inputs are `GENERATION_SEND_TOKENS` (default 1,000), `GENERATION_SEND_WINDOW` (`1` enables history omission), and `GENERATION_SEND_TEMPLATE` (`1` enables a simple real prompt template).

Run the serial size/route matrix when other tests and builds are idle:

```sh
node benchmarks/generation-send/run.mjs
```

Compare two completed coarse matrices with:

```sh
node benchmarks/generation-send/compare.mjs <baseline-results.json> <candidate-results.json>
```

This read-only command verifies matching case sets, coarse probes, recorded host versions, fixture sizes and output counts, then writes median changes to standard output. It rejects fine-probe inputs. It reports when an original exact request hash is unavailable; host CPU contention must be checked separately.

The optional first argument to `run.mjs` selects the output directory. The optional second argument supplies prior result JSON. Matching size, history route and template cases with a captured prompt hash must retain that exact request hash, input tokens, message count, character count, response tokens and persistence route. The output records how many cases had a usable exact reference. Cases absent from the reference still run the persistence and context assertions.

`GENERATION_SEND_REPETITIONS` defaults to three. Each fresh worker runs one case: 100,000, 400,000, or 1,600,000 stored tokens with history omission disabled or enabled. The fixed context limit is 128,000 tokens and the synthetic buffered response is 20,000 tokens. The default history multiplier is two. The fixture reports its actual cl100k_base tokens, characters, UTF-8 bytes, and message count. The global tokenizer string-result cache is disabled because the synthetic messages repeat. Operation-local per-message count reuse follows the real product setting and is visible through the actual encode-call counts. One-time tokenizer WASM initialization is warmed before timing. There are no configured regex/Lua scripts, memory modes, lorebook entries, or output listeners. Default matrix runs have no prompt template; template-enabled runs measure only the explicitly supplied simple template.

The fixture activates the product's metadata-only selected conversation, then verifies the actual persistence route at the provider boundary. The exact response, unchanged history text, message IDs, roles, order, final message count, and context budget are checked against the real store after completion. A configured option alone is not accepted as route evidence.

The dedicated configuration installs timing-only source transforms for history-window preparation, current-chat parsing, prompt selection, history iteration, trimming, template materialization, final token checks, trigger work, the provider request wrapper, output listeners, and tokenization. Complete promotion and IndexedDB commit are timed by forwarding wrappers. The `tokenCalls` field records actual `tokenizeChat` calls and repeated unchanged object/content/name combinations; it does not cache or replace their results. `encodingByZone` records actual encoding time and input characters. If the product still contains eager pre-trim prompt logging, its real stringification executes and the sink counts the resulting string's characters and UTF-8 bytes. That span includes the byte-count probe overhead.

Spans overlap, so their totals must not be added. The entire send interval is split at entry to the synthetic provider. The request wrapper's span includes its preparation before this boundary. Svelte startup roots are delayed until Vitest finishes cyclic module imports, then the real roots run before timing; all subsequent roots run immediately. Product console sinks are suppressed, but arguments computed by product code still execute. Finer probes add overhead, so comparisons should use matching probe configurations.

Results are written to `.tmp/generation-send` by default, including individual logs, numeric observations, and hashes of the measured hot-path sources and fixture/probe files. The runner refuses a matrix whose recorded sources changed during execution. Results describe Node/happy-dom execution and record the host platform and Node version. IndexedDB emulation time is not SQLite or native commit time. The maximum event-loop interval gap includes a one-millisecond sampling interval. Heap samples can miss synchronous peaks and include the Node harness; renderer peak memory is unknown. Native WebView timing, native batch tokenization, rendering, low-spec UI budgets, actual provider/plugin transport, and device crash behavior are unmeasured. Empty script/lorebook/listener branches do not establish the cost of configured workloads. The benchmark can identify preparation costs that warrant investigation, but does not reproduce an iPhone renderer termination.
