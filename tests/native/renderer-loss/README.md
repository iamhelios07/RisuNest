# Windows renderer-loss verification

Run `node tests/native/renderer-loss/run.mjs` on Windows with WebView2 and the repository's native build prerequisites installed. The runner builds a small agent frontend and the existing native boundary executable, then uses CDP `Page.crash` against its hidden WebView.

The seven cases verify retained diagnostic phases, save paths, real SQLite commit outcomes and readback, native document reset, automatic renderer recovery, and exclusion of synthetic private values from the native log. Phase states are synthetic. This does not test interruption of a complete UI send or a provider request.

The post-commit case holds the store caller's completion promise after the real native transport has replied. It independently reads the committed revision, proves the store caller has not observed acknowledgement, and crashes before releasing that promise. This verifies missing acknowledgement at the store caller boundary, not loss of the underlying transport reply.

`--build-only` prepares an immutable executable and prints its path. `--binary=<absolute-path>` runs that executable without rebuilding. Use an executable produced by this runner from the source being verified.

Reports and executable copies are written under `.tmp/test-results/renderer-loss/`. Each run creates a new owned profile and verifies `app_paths_roots` before starting the fixture. The profile is retained and its location is printed. The runner never opens the installed application's profile.
