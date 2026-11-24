#!/usr/bin/env bash
# Run in a normal host terminal: downloads and sudo are required.
set -euo pipefail
umask 077

for command in gh curl tar sha256sum git node npm python3 systemctl sudo getent hostname install; do
  command -v "$command" >/dev/null || { echo "Missing command: $command" >&2; exit 1; }
done

repo=andrzejwitkowski/Bork
data_dir="$HOME/.local/share/bork-review"
runner_dir=/var/lib/bork-review/runner
runner_name="bork-review-$(hostname -s)"
key_file="$HOME/.config/bork-review/opencode-go.key"
test -s "$key_file" || { echo "Missing OpenCode Go key file: $key_file" >&2; exit 1; }
install -d -m 700 "$HOME/.config/bork-review" "$data_dir" "$data_dir/tools"
chmod 600 "$key_file"

# The local profile reads the key at runtime; no raw credential is stored in Git.
npm install --prefix "$data_dir/tools" @alibaba-group/open-code-review@1.14.2
cat > "$HOME/.config/bork-review/review.env" <<'PROFILE'
export PATH="$HOME/.local/share/bork-review/tools/node_modules/.bin:$PATH"
export OCR_LLM_URL=https://opencode.ai/inference/go/openai/v1/chat/completions
export OCR_LLM_MODEL=deepseek-v4.1-flash
export OCR_LLM_PROTOCOL=openai
export OCR_USE_ANTHROPIC=false
export OCR_LLM_TOKEN="$(cat "$HOME/.config/bork-review/opencode-go.key")"
export OCR_LLM_EXTRA_HEADERS="x-opencode-session=bork-local-$(date +%s)"
export OCR_NO_UPDATE=1
ocr config unset provider
ocr config set llm.url "$OCR_LLM_URL"
ocr config set llm.model "$OCR_LLM_MODEL"
ocr config set llm.auth_token ''
ocr config set llm.auth_token_cmd 'printf "%s" "$OCR_LLM_TOKEN"'
ocr config set llm.auth_header ''
ocr config set llm.extra_headers ''
ocr config set llm.protocol "$OCR_LLM_PROTOCOL"
ocr config set llm.extra_body '{"thinking":{"type":"disabled"}}'
PROFILE
chmod 600 "$HOME/.config/bork-review/review.env"
( source "$HOME/.config/bork-review/review.env"; ocr llm test )

if ! gh api --method PUT "repos/$repo/actions/permissions/fork-pr-contributor-approval" \
  -f approval_policy=all_external_contributors; then
  echo "Could not configure fork approval for $repo. Run setup with a GitHub CLI login that has repository administrator access." >&2
  exit 1
fi
gh secret set OCR_LLM_AUTH_TOKEN --repo "$repo" < "$key_file"

sudo -v
if ! id bork-review >/dev/null 2>&1; then
  sudo useradd --system --user-group --create-home --home-dir /var/lib/bork-review --shell /usr/sbin/nologin bork-review
fi
if [ "$(getent passwd bork-review | cut -d: -f6)" != /var/lib/bork-review ]; then
  echo 'The existing bork-review account has an unexpected home directory' >&2
  exit 1
fi
sudo install -d -m 700 -o bork-review -g bork-review /var/lib/bork-review
sudo install -d -m 700 -o bork-review -g bork-review "$runner_dir"
if ! sudo test -x "$runner_dir/bin/Runner.Listener"; then
  archive="$data_dir/actions-runner.tar.gz"
  curl --fail --location --retry 3 \
    https://github.com/actions/runner/releases/download/v2.338.2/actions-runner-linux-x64-2.338.2.tar.gz \
    --output "$archive"
  echo "baebf204307bac6193480852732f0aba2ae97b6c5597c244f4d5558390147c40  $archive" | sha256sum --check --status
  sudo -u bork-review tar -xzf - -C "$runner_dir" < "$archive"
  rm "$archive"
fi

sudo -u bork-review install -m 700 "$runner_dir/bin/runsvc.sh" "$runner_dir/runsvc.sh"

if ! sudo test -f "$runner_dir/.runner"; then
  gh api --method POST "repos/$repo/actions/runners/registration-token" --jq .token | \
    sudo -H -u bork-review env -u OCR_LLM_TOKEN PATH=/usr/local/bin:/usr/bin:/bin \
    bash -ec 'cd "$1"; shift; IFS= read -r ACTIONS_RUNNER_INPUT_TOKEN; export ACTIONS_RUNNER_INPUT_TOKEN; exec ./config.sh "$@"' bash "$runner_dir" \
    --unattended --url "https://github.com/$repo" \
    --name "$runner_name" --labels bork-review --work _work
fi
if ! runner_id="$(sudo -u bork-review python3 -c 'import json, sys
with open(sys.argv[1], encoding="utf-8") as registration:
    runner_id = json.load(registration)["agentId"]
if type(runner_id) is not int or runner_id <= 0:
    raise ValueError("expected a positive runner ID")
print(runner_id)
' "$runner_dir/.runner" 2>/dev/null)"; then
  echo "Cannot read runner ID from $runner_dir/.runner. Check the registration file and re-register the runner if needed." >&2
  exit 1
fi

sudo tee /etc/systemd/system/bork-review-runner.service >/dev/null <<UNIT
[Unit]
Description=Bork GitHub Actions review runner
After=network-online.target

[Service]
User=bork-review
Group=bork-review
WorkingDirectory=$runner_dir
ExecStart=$runner_dir/runsvc.sh
Environment=PATH=/usr/local/bin:/usr/bin:/bin
Restart=always
RestartSec=10
UMask=0077
ProtectHome=true
ProtectSystem=strict
ReadWritePaths=/var/lib/bork-review
PrivateTmp=true
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
UNIT
sudo systemctl daemon-reload
sudo systemctl enable --now bork-review-runner.service
sudo systemctl is-active bork-review-runner.service
for attempt in {1..20}; do
  status="$(gh api --paginate "repos/$repo/actions/runners" --jq ".runners[] | select(.id == $runner_id) | .status")"
  if [ "$status" = online ]; then
    echo 'Runner online. Once the workflow is on the default branch, comment /open-code-review on an open PR.'
    exit 0
  fi
  sleep 3
done
echo "Runner ID $runner_id did not come online; inspect sudo journalctl -u bork-review-runner.service. If removed on GitHub, run config.sh remove and register again." >&2
exit 1
