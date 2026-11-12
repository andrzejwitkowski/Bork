# MIR backend benchmark (RFC 0006 M6, #67)

Pre-MIR baseline against the current compiler, `tools/bench_quicksort.sh BASE_BORK HEAD_BORK [runs]`.

- **Baseline:** `c33c0e4`, the last commit with the HIR emitter as the default backend, built with
  `cargo build --release --features codegen`.
- **Head:** the compiler after the HIR emitter was deleted (this branch), same build flags.
- **Budget (RFC 0006 section 7):** `bork build` no more than 1.5x the baseline; emitted-code speed not slower.
- Median of 15 runs, wall clock, same machine, `clang` as linker. Output and exit code of each program matched
  between the two compilers.

| program | build base (s) | build head (s) | ratio | run base (s) | run head (s) | ratio |
|---|---|---|---|---|---|---|
| `tools/bench/sort_loop.bork` | 0.117 | 0.111 | 0.95 | 0.0571 | 0.0652 | 1.14 (noise, below) |
| `tools/bench/quicksort_simple.bork` | 0.113 | 0.110 | 0.97 | 0.0039 | 0.0039 | 1.00 |
| `tools/bench/quicksort.bork` | 0.131 | 0.116 | 0.89 | 0.0040 | 0.0040 | 0.99 |
| `tools/bench/quicksort_ref.bork` | 0.117 | 0.115 | 0.98 | 0.0039 | 0.0040 | 1.01 |
| `tools/bench/quicksort_rec.bork` | 0.133 | 0.125 | 0.94 | 0.0039 | 0.0039 | 1.01 |

Reading it:

- **Build time** is 0.89 to 0.98 of the baseline: inside the budget, with room.
- **Run time of the quicksort programs** is about 4 ms, which is process start, so they only show the output is
  unchanged. `sort_loop` (bubble sort of 400 elements, 300 rounds, about 60 ms) is the one that measures emitted code.
  Its first median read 1.14; three further batches of 40 runs gave 0.86, 1.07 and 0.99, so the difference is
  within run-to-run noise and no regression is shown. A longer kernel would narrow the noise further.
- Not measured: `-g` builds, programs with `Ref`/`Vec` traffic, peak memory.
