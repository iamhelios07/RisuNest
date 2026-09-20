# Synthetic UI responsiveness benchmark

Mounts the production chat list and body with two deterministic in-memory characters. It does not start the application bootstrap, open a database, or read a backup.

Compare the pinned baseline and working tree from the repository root:

```powershell
node benchmarks/ui-responsiveness/runner.mjs --source baseline --samples 5
node benchmarks/ui-responsiveness/runner.mjs --source candidate --samples 5
```

The runner uses a fresh Edge or Chrome profile, blocks RisuRealm before navigation, rejects non-loopback requests, and verifies mounted content hashes. Set `RISUNEST_UI_BENCH_BROWSER` when browser discovery is unsuitable.

Reported durations cover module load, first verified chat render, navigation, and combined readiness. They are component measurements, not full application or database startup.

Add `--screenshot` for optional synthetic visual QA. Run the separate navigation-overlay check with:

```powershell
node benchmarks/ui-responsiveness/visual-runner.mjs
```

The visual runner verifies overlay bounds, spinner placement, and inert content. It does not mount the full app shell or native window.
