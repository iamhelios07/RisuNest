# Lua Worker asset

Use the repository's Node and pnpm versions and install with `pnpm install --frozen-lockfile`.

Run `pnpm lua:build-worker` to generate `src/ts/process/lua/worker.wasm` from the pinned Wasmoon 1.16.0 WASM using Binaryen 123.0.0. Run `pnpm lua:check-worker` to verify the checked-in asset without changing it. The build checks the input SHA-256 and prints the output size and SHA-256.

The Worker requires the Wasmoon patch in `patches/wasmoon@1.16.0.patch`. The asset supports Lua 5.4 and suspends synchronous host calls with Asyncify, including calls inside Lua C-library callbacks. It uses ordinary Worker messages and does not require shared memory or cross-origin isolation. Each retained engine has separate Lua globals and WASM memory; engines share a Worker.

Run the scripting, Lua runtime, JSON and Worker tests after changing the patch, transform or host protocol. Run `pnpm check:production-bundle` after changing Worker entries or asset paths. Pure computation and frequent small host calls can take longer than the main-thread runtime; compare execution time and UI responsiveness separately.

The distributed Wasmoon and Lua notices are in `public/licenses/lua-worker.txt`.
