# Automatic strict code review

OpenCodeReview runs on a dedicated **self-hosted Linux x64 runner** labelled
`bork-review`. The selected model is **DeepSeek V4.1 Flash through OpenCode Go**.
The machine runs OCR and the Actions runner; inference uses the Go API.

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

## 1. Start the configured machine

The API key belongs in `~/.config/bork-review/opencode-go.key`, mode `0600`,
not in this repository. It is also stored as the encrypted Actions secret
`OCR_LLM_AUTH_TOKEN`. The workflow defaults already select the Go endpoint,
`deepseek-v4.1-flash`, the OpenAI protocol, and Go's required user-agent/session
headers. There are no model variables or URL secrets to configure manually.

Run in a normal terminal on the machine:

```sh
bash scripts/setup-review-runner.sh
```

The installer:

1. Installs OCR 1.14.2 under `~/.local/share/bork-review/tools` and writes a
   credential-reading local profile at `~/.config/bork-review/review.env`.
2. Runs `ocr llm test` against Go before registering anything.
3. Uploads the local key as an encrypted GitHub Actions secret using the existing
   `gh` login.
4. Downloads the pinned Linux x64 runner 2.338.2, verifies its SHA-256 checksum,
   and registers it automatically with the `bork-review` label.
5. Creates a dedicated `bork-review` system account and enables the system service
   `bork-review-runner.service`. Sudo is needed to create the account and service;
   the runner starts at boot and survives logout.

The machine needs Git >= 2.41, Node/npm (the local OCR launcher supports Node >= 14),
Python 3, curl, tar, GitHub CLI authenticated with repository admin access, sudo,
and systemd. Each CI job sets up Node 24 itself. The service runs under the
dedicated account in `/var/lib/bork-review`; systemd blocks access to personal
home directories, limits writes to that runner directory and private temporary
files, and prevents privilege elevation. Personal Go/gh credentials stay with
the invoking user; jobs receive their model credential from Actions secrets.
The script must run outside a restricted agent sandbox because it downloads
binaries, calls Go, registers the runner and starts the system service.

Check readiness with:

```sh
systemctl status bork-review-runner.service
```

For manual local reviews, load the prepared profile first:

```sh
source ~/.config/bork-review/review.env
ocr review
```

The machine must remain on and connected. Keep one runner service per machine:
the upstream Action uses fixed `/tmp/ocr-result.json` and `/tmp/ocr-stderr.log`
paths, and a single runner executes jobs serially.

Bork is public. Repository runners can also be selected by workflows modified
in fork PRs, so the installer enforces approval for **all external contributors**,
not only new contributors. This repository setting is already configured.
Inspect workflow changes before approving external runs; do not approve a fork
workflow that executes arbitrary PR code on `bork-review`. The trusted
`pull_request_target` reviewer still reads fork source as data.

## 2. Model settings

The default full endpoint is
`https://opencode.ai/inference/go/openai/v1/chat/completions` and the model ID is
`deepseek-v4.1-flash`. OCR sends its own `User-Agent` automatically; do not add it
to extra headers, where OCR rejects reserved headers. Both local and CI requests
add a stable `x-opencode-session` for each review, as required by
[OpenCode Go](https://opencode.ai/docs/console/go/).

The supplied key needs an active Go subscription with access to DeepSeek. Go may
require Global regions enabled in the workspace's Privacy settings; a rejected
model-access test must be resolved there before the installer continues.

Optional overrides remain available through the `OCR_LLM_URL` secret and
`OCR_LLM_MODEL`, `OCR_LLM_PROTOCOL`, `OCR_LLM_EXTRA_BODY` repository variables.
The local profile is specific to the selected Go account/model.

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
