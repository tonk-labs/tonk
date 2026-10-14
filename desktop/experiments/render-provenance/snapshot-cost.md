# Overlay snapshot cost checkpoint

The query candidate now uses copy-on-write overlay snapshots. All 466 repository
tests passed after the change (one manual benchmark ignored), including pinned
durable/overlay races and mutation of a retained overlay snapshot. Formatting
and whitespace checks passed.

`snapshot-cost.csv` contains eight paired samples per case, alternating eager
copy and COW order, with 20 iterations per sample. The fact set stays fixed at
initial facts plus one counter; the bounded instant log is warmed before timing.
The native optimized test binary was used. Other compilation was active, and
sample variability is substantial; these are cost diagnostics, not a calibrated
application benchmark.

For 10,000 initial facts:

| Operation | Eager copy median (range) | COW median (range) |
| --- | --- | --- |
| Capture/read/drop | 15.71 ms (13.77–18.61) | 0.09 us (0.035–0.258) |
| Capture/write/read/drop, snapshot held | 10.22 ms (7.80–18.02) | 9.90 ms (8.04–13.07) |

Unchanged snapshots share the indexes and bounded log instead of copying them.
A write while a snapshot is retained still copies that state; there is no
established write-path speedup. Subscriptions retain their pinned branch between
polls, so that copy can happen outside the active polling interval too. Peak
memory and end-to-end native/browser build-loop latency remain unmeasured.

Reproduce in the patched dialog-db checkout with:

```sh
cargo test -p dialog-repository --lib repository::
cargo test -p dialog-repository --lib measure_read_snapshot_cost -- --ignored --nocapture
```

## Separate-target rerun

After all Tonk compilation finished, the dialog-db workspace rebuilt in its own
target directory. All 466 repository tests and the benchmark passed. Raw samples
are in `snapshot-cost-isolated.csv`. The same method produced:

| Operation, 10,000 initial facts | Eager copy median (range), us | COW median (range), us |
| --- | --- | --- |
| Capture/read/drop | 2072.395 (1929.937–2341.964) | 0.030 (0.016–0.052) |
| Capture/write/read/drop, snapshot held | 1806.102 (1761.318–2512.910) | 1855.420 (1787.112–1995.539) |

These are still native microbenchmarks, not end-to-end latency or peak-memory
measurements. Both runs support avoiding eager read copies; neither establishes a
write-path speedup. No product performance claim is made.
