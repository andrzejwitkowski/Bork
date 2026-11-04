# AGENTS.md

## Running tests (the full suite can crash the machine)

Plain `cargo test --features codegen` takes the machine down (parallel LLVM compiles and test
processes). Run it throttled and in the background, with output to a file:

```sh
export LD_LIBRARY_PATH=/home/enrju/llvm23-prefix/usr/lib/x86_64-linux-gnu   # libz3.so.4 for the LLVM 23 build
nice -n 19 cargo test -j1 --features codegen -- --test-threads=1 > /path/to/log 2>&1 &
```

- Run a narrow target (`--lib <filter>`, `--test <name>`) instead of the whole suite when possible.
- Without `LD_LIBRARY_PATH` the test binary fails with `libz3.so.4: cannot open shared object file`.
- Tests that link executables need `clang` on `PATH`; without it they fail with
  `failed to run clang: No such file or directory`. That is an environment problem, not a code failure.
