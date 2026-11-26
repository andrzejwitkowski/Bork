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

## Review comments on a PR (every thread gets an answer)

Before you tell the user a PR is done, no review thread may be left silently open. List them with
`gh api graphql` (`reviewThreads`: `id`, `isResolved`, `path`, `line`, first comment) and for each one:

- **Fixed:** reply with the commit SHA (`fixed in <sha>`), then resolve the thread.
- **Wrong or not applicable:** reply with the concrete reason (the code that disproves it), then resolve.
  Verify the claim in the code first; bots confuse AST control flow with checker control flow.
- **Valid but deferred:** reply with the issue that tracks it (`tracked in #N`), create that issue if none exists,
  then resolve. Never defer without an issue number.

Rules:
- Do this after every push: a new push can add new threads (OpenCodeReview and CodeRabbit re-review each head).
- Check CI against the PR's *current* head SHA (`gh pr view <n> --json headRefOid`, `gh run list --branch <b>`), not
  the first red result you see; a push that GitHub did not register leaves the old runs on the old SHA.
- When you report status, say per thread what happened (fixed / rejected / deferred, with SHA or issue), and which
  CI checks are green on which SHA. Do not write "addressed" without one of those.
- Run the **whole** `--lib` target before pushing, not a `--lib <filter>`: a layout corpus test that the filter skipped
  is what turned CI red on #354. Re-run the golden targets after any change to comments or prelude source too (spans move).
