# Performance record

These measurements are engineering evidence, not CI thresholds. They were
captured on 2026-08-29 on an Apple M5 MacBook Pro with 16 GB of memory, macOS
26.6, Homebrew 6.0.20, and an optimized Rust build. The local Homebrew cache
contained 16,291 packages (8,575 formulae and 7,716 casks).

## Baseline

Revision `74afd5b` scored 16,291 synthetic package names in **814.458 us** in a
release unit test. The original debug test took **6.899 ms**. This measured only
name scoring and counted hits; it did not build stable identities, search
descriptions, or rank and return results. A cold build of the original test
suite took **49.93 seconds** and peaked at **1.56 GB** RSS.

## Reconstructed application

Criterion's release benchmark scans and ranks the complete 16,291-entry real
catalog, including name and description projections:

| Query | Criterion estimate |
| --- | ---: |
| `pnt` | 1.3469-1.3679 ms |
| `brew utility` | 860.32-871.49 us |
| `package-name-15999` | 861.64-873.70 us |

The broader post-refactor workload is not directly interchangeable with the
old score-only test. Both are reported to avoid implying a false speedup. The
current end-to-end search remains comfortably below an 8.33 ms 120 Hz input
budget on this machine.

`cargo run --release --example perf_probe` measured:

| Operation | Result |
| --- | ---: |
| Direct installed scan (1,094 packages) | 116.879 ms |
| Validated private-cache catalog load | 15 ms |
| Catalog-to-application projection | 3 ms |
| Empty Browse transition (`Arc` view reuse) | p50 0 ns / p95 42 ns |
| `git` search | p50 1.244 ms / p95 1.312 ms |
| Authoritative `brew outdated --json=v2` | 2.315 s |
| Probe peak RSS | 153.9 MiB |

The final optimized, ad-hoc-signed native-app smoke run reported **286 ms**
from process entry to window creation. At 19 seconds, after the package views
were populated, process RSS was **124.9 MiB**. The refresh state machine reached
idle after **2.849 s** total (installed preview 156 ms, catalog preview 174 ms,
authoritative outdated result 2.847 s; stage values are cumulative). The
finished UI showed 1,106 installed packages, 4 outdated packages, and the
complete 16,291-package catalog. Application RSS includes GPUI/Metal resources
and is not directly comparable to the headless probe's peak.

The outdated subprocess is intentionally not placed on the first-paint path.
Use `KETTLE_PERF_LOG=1 cargo run --release` to print the time from application
entry to successful GPUI window creation. Use the performance probe for data
pipeline timings and Criterion for statistically useful search measurements.

## Reproduction

```sh
cargo bench --bench search
cargo run --release --example perf_probe
KETTLE_PERF_LOG=1 cargo run --release
```

Absolute timings vary with hardware, filesystem cache state, catalog contents,
and Homebrew network/update activity. Tests assert deterministic behavior;
benchmarks report latency without flaky absolute pass/fail limits.
