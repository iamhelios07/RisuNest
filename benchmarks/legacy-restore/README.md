# Synthetic legacy restore measurements

Run the Windows entry-count matrix from a checkout with its dependencies installed:

```text
node benchmarks/device-backup/windows-smoke.mjs --legacy-entries=all
```

The runner builds an isolated agent executable and restores 20,000, 50,000,
100,000 and 200,001 assets with 384-byte names and 32-byte bodies. Each case
starts a fresh native process and profile. It measures native working set and
renderer heap, verifies every restored name, hash and body, and checks that a
truncated restore preserves the active root and revision. Results include the
source and executable hashes. Earlier synthetic profiles are retained at the
paths recorded in the result.

Use `--legacy-entries=20000` (or another matrix count) for one case. These
measurements are separate from normal unit tests. The existing decoded-size
axis remains available through the Android and iOS harnesses.
