# Official account measurements

Run `node tests/native/run.mjs --official-account-benchmark --debug` from a Windows, macOS or Linux checkout with the native Tauri prerequisites installed. Omit `--debug` for a release build. The runner uses the shared Cargo target, a temporary synthetic app profile and a loopback HTTP server. It never signs in to an account.

The report covers small and multi-MiB bodies at injected request latencies with concurrency 1, 2 and 4, publication-ledger serialization and native persistence, batched asset residency, frozen continuation manifests, scheduling costs, and real elapsed checkpoints. Counts reach 100,000 separately from the body workloads. Stored aliases, manifests and ledger contents are checked against synthetic expectations. Results appear under `.tmp/test-results/native`.

The account transport boundary uses a synthetic HTTP adapter; scheduling, installation, CAS, SQLite and ledger behavior use product modules. Timings can nest and must not be added indiscriminately. JS heap samples exclude native process memory. Debug results are not release performance. Neither local throughput nor injected errors establish an official server concurrency allowance.

`--runs=2` repeats the run. `--reuse-build` requires the same previously built benchmark binary. Use `--reuse-build --binary=<saved-executable>` to compare a preserved executable. The runner rejects build reuse outside benchmark mode.
