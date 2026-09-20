# W8 bundle gates

Measures the Roadmap 14 Highlight and SortableJS bundle gates without contacting RisuRealm.

## Initial production graph

Build with a Vite manifest and source maps, then measure the static entry closure:

    pnpm exec vite build --manifest .vite/manifest.json --outDir dist --sourcemap
    node benchmarks/w8/bundle-gates.mjs --dist dist

Dynamic imports are excluded. The report lists raw and gzip bytes and identifies Highlight or SortableJS code in the startup graph.

## Highlight split candidate

highlight-candidate.patch is the retained build-only candidate. It is evidence, not a proposed product change. Reproduce it in an isolated worktree:

    pwsh -NoProfile -File benchmarks/w8/reproduce-highlight-candidate.ps1

The script verifies the patch hash, creates a detached worktree, builds and measures the candidate, and removes only its owned generated files and worktree.

Measure first Highlight use separately:

    $w8Output = Join-Path $env:TEMP 'risunest-w8-highlight-measurement'
    pnpm exec vite build --config benchmarks/w8/vite.highlight.config.ts --outDir $w8Output --emptyOutDir
    node benchmarks/w8/highlight-first-use.mjs --dist $w8Output --samples 20

Set RISUNEST_W8_BROWSER when Edge or Chrome discovery is unsuitable.

## SortableJS upper bound

This build replaces SortableJS imports with a no-op build-only shim. It measures the maximum initial graph saving and is not suitable for application testing or distribution.

    pnpm exec vite build --config benchmarks/w8/vite.sortable-upper-bound.config.ts --manifest .vite/manifest.json --outDir dist --sourcemap
    node benchmarks/w8/bundle-gates.mjs --dist dist

Retained results and interpretation live under docs/native-app-analysis/roadmap-14-implementation/implementation-reports/.
