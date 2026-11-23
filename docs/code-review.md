# Automatic strict code review

OpenCodeReview runs on a dedicated **self-hosted Linux x64 runner** labelled
`bork-review`. The runner can call a model on the same machine over localhost;
GitHub does not need an inbound connection to the machine or model port.

## What runs

- PR creation, new commits, reopening, edits (including base changes), and marking
  a draft ready: full PR review, inline comments and an updated summary.
- Push to a branch without an open PR: review the pushed range and save the JSON
  result and diagnostic log as an Actions artifact. New branches are compared
  with the default branch. Non-fast-forward pushes and unavailable previous
  commits trigger a full repository scan instead of a misleading merge-base diff.
- Push to a branch with an open PR: the PR event handles review, avoiding a second
  model invocation from the push event.
- A PR comment starting with `/open-code-review` or `@open-code-review`: re-review,
  restricted to human owners, members and collaborators.
- **Actions → OpenCodeReview → Run workflow**: enter an open PR number to re-review.

The workflow pins the Action and CLI to **1.14.2**. There is no OCR option named
`strict`: this configuration uses `effort: high`, full-range review on every run,
one concurrent task, a 30-minute task timeout, a 120-minute job timeout, and a
2,000,000-token budget. Comment deduplication does not reduce review coverage.
Rules preserve OCR's built-in checks and add Bork's compiler/runtime invariants.
`.bork` programs, expected outputs, Rust integration tests and documentation are
explicitly included instead of falling through OCR's default exclusions.

The **`OpenCodeReview / strict (main)` commit status** fails for high/critical findings,
incomplete or waived coverage, files too large to review, token/round limits,
invalid output, or an operational failure. Medium/low findings remain visible
without failing the status. A run with no eligible changed files may skip cleanly.
This status is attached to the reviewed **head commit**, including for
`pull_request_target`; the workflow's ordinary job check alone is not the gate.
Other PR base branches get their own `strict (<base>)` status; pushes without a PR
use `OpenCodeReview / push`. A clean review against another base or a smaller push
range cannot overwrite a failed PR review on the same commit.

## 1. Register the machine

Use a dedicated OS account and runner work directory, separate from your personal
checkout and credentials. The runner must have Bash, **Git >= 2.41**, **Node.js 24
with npm**, and **Python 3** on its service user's PATH. OCR is installed by the
workflow into the runner's temporary directory; no global privileged npm install
or Rust/LLVM build is needed for review.

1. Open [Bork → Settings → Actions → Runners](https://github.com/andrzejwitkowski/Bork/settings/actions/runners).
2. Select **New self-hosted runner**, then **Linux / x64**.
3. Under the dedicated account, follow GitHub's generated download and
   configuration commands. Add `--labels bork-review --name bork-review` to the
   generated `./config.sh` command. Keep the one-time registration token private.
4. Start `./run.sh` and confirm that GitHub displays the runner as **Idle**.
5. For unattended operation, install/start the runner's service using the
   generated runner directory's `svc.sh`, selecting the dedicated account.
   Verify that Node/npm and the model remain available when the service runs.

Keep one runner service per machine for this workflow: the upstream composite
Action uses fixed `/tmp/ocr-result.json` and `/tmp/ocr-stderr.log` paths. A runner
executes one job at a time; separate concurrent services would share those files.
The machine must remain on and connected, otherwise jobs wait for its runner.

## 2. Configure the model

The Actions runner executes OCR; **it does not start a model server**. Start your
existing OpenAI-compatible model server separately and enable tool/function
calling. OCR needs that capability to read and search repository context.

For example, a model listening on port 8080 could expose:

```text
http://127.0.0.1:8080/v1/chat/completions
```

Use the actual port and model ID from your server; `GET /v1/models` normally lists
available IDs. The example URL is the full completion endpoint, not just `/v1`.
The model needs enough context for the selected files and review rules.

In [Settings → Secrets and variables → Actions](https://github.com/andrzejwitkowski/Bork/settings/secrets/actions), set:

| Kind | Name | Value |
| --- | --- | --- |
| Secret | `OCR_LLM_URL` | Full endpoint URL reachable from the runner, e.g. the localhost URL above |
| Secret | `OCR_LLM_AUTH_TOKEN` | Model API token; a nonempty dummy value such as `local` works only if your localhost server does not require authentication |
| Variable | `OCR_LLM_MODEL` | Exact model ID accepted by the server |
| Variable, optional | `OCR_LLM_PROTOCOL` | `openai` (default), `openai-responses`, or `anthropic` |
| Variable, optional | `OCR_LLM_EXTRA_BODY` | Provider-specific JSON, e.g. thinking settings; default `{}` |

The same inputs also support a remote model API. A paid coding-agent subscription
does not itself supply this workflow's model credentials.

## 3. Enable and verify

Merge the workflow and rules into the default branch first. PR-target and comment
events use trusted base-branch files, so they cannot use this setup while it exists
only in an unmerged PR. The new workflow can already run for pushes of its branch
once the runner and model configuration are present.

After merging, comment `/open-code-review` on an existing PR or use the manual
workflow trigger. Confirm that the model is called, a summary is posted, and the
`OpenCodeReview / strict (main)` status appears on the PR's latest commit. A red result
can mean findings or incomplete review; inspect the summary and JSON artifact.

To block merges on the result, add **`OpenCodeReview / strict (main)`** as a required
status check in the branch ruleset/protection for `main` after its first run.
Do not select only the ordinary `Strict review` Actions job: a duplicate push
event intentionally skips its model work and may finish green.

Disable CodeRabbit for Bork after confirming the replacement works. Removing its
repository access and cancelling any paid subscription are separate account
settings; this PR does neither.

## Trust boundary and local checks

For public fork PRs, the pinned upstream Action checks out the trusted base and
reads PR-head Git objects. This workflow never builds, tests, or executes PR-head
code with secrets on the self-hosted runner. Keep that property when editing it.
The push path checks out only branches pushed by repository writers. Model access
is read-only review; no fixes or automatic merges are performed.

Run the strict-gate tests without a model:

```sh
python3 -m unittest discover -s scripts -p test_ocr_review.py -v
```

With OCR installed, verify selection/rules without spending model tokens:

```sh
ocr review --from main --preview
ocr rules check crates/bork_runtime/src/lib.rs
ocr rules check tests/build.rs
```

Upstream references: [GitHub Actions integration](https://github.com/alibaba/open-code-review/tree/v1.14.2/examples/github_actions),
[Action inputs](https://github.com/alibaba/open-code-review/blob/v1.14.2/action.yml),
and [file selection and rules](https://github.com/alibaba/open-code-review/blob/v1.14.2/pages/src/content/docs/en/review-rules.md).
