"""Exercise the strict gate using OCR's JSON coverage and finding contract."""

import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest


GATE = Path(__file__).with_name("ocr_review.py")
COMPLETE = {
    "status": "complete",
    "manifest": {
        "terminal_state": "complete",
        "coverage": {
            "selected": [{"item_id": "runtime", "path": "crates/bork_runtime/src/lib.rs"}],
            "completed": [{"item_id": "runtime", "path": "crates/bork_runtime/src/lib.rs"}],
            "reused": [], "failed": [], "waived": [], "excluded": [],
        },
    },
    "summary": {"budget_exceeded": False},
    "comments": [],
}


class StrictReviewTests(unittest.TestCase):
    def check_gate(self, result, expected):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.json"
            path.write_text(json.dumps(result), encoding="utf-8")
            env = dict(os.environ)
            env.pop("GITHUB_STEP_SUMMARY", None)
            process = subprocess.run(
                [sys.executable, str(GATE), str(path)],
                capture_output=True, text=True, env=env,
            )
        self.assertEqual(process.returncode, expected, process.stdout + process.stderr)
        self.assertTrue(process.stdout.startswith("Strict review:"), process.stdout + process.stderr)

    def test_complete_review_allows_medium_and_low_findings(self):
        result = copy.deepcopy(COMPLETE)
        result["comments"] = [{"severity": "medium"}, {"severity": "low"}]
        self.check_gate(result, 0)

    def test_high_and_critical_findings_fail(self):
        for severity in ("high", "critical"):
            with self.subTest(severity=severity):
                result = copy.deepcopy(COMPLETE)
                result["comments"] = [{"severity": severity}]
                self.check_gate(result, 1)

    def test_unclassified_findings_fail_closed(self):
        for comment in ({}, {"severity": None}, {"severity": ""},
                        {"severity": "unknown"}, {"severity": 1}):
            with self.subTest(comment=comment):
                result = copy.deepcopy(COMPLETE)
                result["comments"] = [comment]
                self.check_gate(result, 1)

    def test_partial_or_failed_review_cannot_pass_without_findings(self):
        for status in ("partial", "failed"):
            with self.subTest(status=status):
                result = copy.deepcopy(COMPLETE)
                result["status"] = result["manifest"]["terminal_state"] = status
                self.check_gate(result, 1)

    def test_unreviewed_or_waived_files_fail_even_with_complete_status(self):
        for destination in ("failed", "waived"):
            with self.subTest(destination=destination):
                result = copy.deepcopy(COMPLETE)
                coverage = result["manifest"]["coverage"]
                coverage[destination] = coverage.pop("completed")
                self.check_gate(result, 1)

    def test_wrong_completed_file_cannot_satisfy_coverage(self):
        result = copy.deepcopy(COMPLETE)
        result["manifest"]["coverage"]["completed"][0]["item_id"] = "another-file"
        self.check_gate(result, 1)

    def test_budget_or_round_limit_is_not_a_clean_review(self):
        result = copy.deepcopy(COMPLETE)
        result["summary"]["budget_exceeded"] = True
        self.check_gate(result, 1)
        result = copy.deepcopy(COMPLETE)
        result["warnings"] = [{"type": "review_limit_reached"}]
        self.check_gate(result, 1)

    def test_no_reviewable_files_is_a_valid_skip(self):
        result = copy.deepcopy(COMPLETE)
        result["status"] = result["manifest"]["terminal_state"] = "skipped"
        result["manifest"]["coverage"]["selected"] = []
        result["manifest"]["coverage"]["completed"] = []
        self.check_gate(result, 0)

    def test_skip_cannot_hide_selected_files(self):
        result = copy.deepcopy(COMPLETE)
        result["status"] = result["manifest"]["terminal_state"] = "skipped"
        self.check_gate(result, 1)

    def test_file_excluded_for_size_is_not_a_successful_skip(self):
        result = copy.deepcopy(COMPLETE)
        result["status"] = result["manifest"]["terminal_state"] = "skipped"
        coverage = result["manifest"]["coverage"]
        coverage["selected"] = coverage["completed"] = []
        coverage["excluded"] = [{"path": "src/large.rs", "exclude_reason": "too_large"}]
        self.check_gate(result, 1)

    def test_missing_coverage_fields_fail_closed(self):
        result = copy.deepcopy(COMPLETE)
        result["manifest"]["coverage"] = {}
        self.check_gate(result, 1)

    def test_missing_manifest_or_malformed_result_fails_closed(self):
        for result in ({"status": "success", "comments": []}, [], None):
            with self.subTest(result=result):
                self.check_gate(result, 1)


class WorkflowTriggerTests(unittest.TestCase):
    def test_push_is_skipped_only_when_an_authorized_pr_event_can_review_it(self):
        workflow = GATE.parents[1] / ".github/workflows/ocr-review.yml"
        marker = "      - name: Resolve review context\n"
        workflow_text = workflow.read_text()
        self.assertIn(marker, workflow_text, "Review context step is missing or renamed")
        step = workflow_text.split(marker, 1)[1]
        script = textwrap.dedent(step.split("\n      - name:", 1)[0].split("          script: |\n", 1)[1])
        harness = """
            const prs = JSON.parse(process.argv[1]);
            const outputs = {};
            const context = {eventName: 'push', payload: {after: 'head'},
                ref: 'refs/heads/feature', repo: {owner: 'owner', repo: 'repo'}};
            const core = {notice: () => {}, setOutput: (key, value) => outputs[key] = value};
            const github = {rest: {pulls: {list: async () => ({data: prs})}},
                paginate: async (_list, options) => {
                    if (options.state !== 'open' || options.head !== 'owner:feature') {
                        throw new Error('Push lookup must filter open PRs by head branch');
                    }
                    return prs;
                }};
        """ + "(async () => {" + script + "})().then(() => console.log(JSON.stringify(outputs)));"
        for associations, expected_mode in [([], "push"), (["NONE"], "push"),
                                            (["OWNER"], None), (["NONE", "COLLABORATOR"], None)]:
            with self.subTest(associations=associations):
                prs = [{"author_association": association} for association in associations]
                output = subprocess.check_output(["node", "-e", harness, json.dumps(prs)], text=True)
                self.assertEqual(json.loads(output).get("mode"), expected_mode)

    def test_only_authorized_events_start_the_runner(self):
        workflow = GATE.parents[1] / ".github/workflows/ocr-review.yml"
        condition = workflow.read_text().split("    if: >-\n", 1)[1].split("\n    runs-on:", 1)[0]
        script = """
            const github = JSON.parse(process.argv[1]);
            const fromJSON = JSON.parse;
            const contains = (items, value) => items.includes(value);
            const startsWith = (value, prefix) => value.startsWith(prefix);
        """ + "console.log(Boolean(" + condition + "));"
        cases = [
            ("pull_request_target", {"pull_request": {"author_association": "OWNER"}, "action": "opened"}, True),
            ("pull_request_target", {"pull_request": {"author_association": "COLLABORATOR"}, "action": "synchronize"}, True),
            ("pull_request_target", {"pull_request": {"author_association": "NONE"}, "action": "opened"}, False),
            ("pull_request_target", {"pull_request": {"author_association": "OWNER"}, "action": "edited", "changes": {"title": {}}}, False),
            ("pull_request_target", {"pull_request": {"author_association": "OWNER"}, "action": "edited", "changes": {"base": {"ref": {}}}}, True),
            ("push", {}, True),
            ("workflow_dispatch", {}, True),
        ]
        for association, user_type, body, state, allowed in [
            ("OWNER", "User", "/open-code-review", "open", True),
            ("MEMBER", "User", "@open-code-review", "open", True),
            ("NONE", "User", "/open-code-review", "open", False),
            ("COLLABORATOR", "Bot", "/open-code-review", "open", False),
            ("OWNER", "User", "unrelated comment", "open", False),
            ("OWNER", "User", "/open-code-review-disabled", "open", False),
            ("MEMBER", "User", "@open-code-reviewer", "open", False),
            ("OWNER", "User", "/open-code-review please", "open", True),
            ("OWNER", "User", "/open-code-review", "closed", False),
        ]:
            cases.append(("issue_comment", {
                "issue": {"pull_request": {}, "state": state},
                "comment": {"author_association": association, "user": {"type": user_type}, "body": body},
            }, allowed))
        for event_name, event, allowed in cases:
            with self.subTest(event_name=event_name, event=event):
                output = subprocess.check_output(
                    ["node", "-e", script, json.dumps({"event_name": event_name, "event": event})], text=True,
                )
                self.assertEqual(output.strip(), str(allowed).lower())


class PushRangeTests(unittest.TestCase):
    def test_pushes_do_not_turn_rewinds_into_empty_reviews(self):
        workflow = GATE.parents[1] / ".github/workflows/ocr-review.yml"
        marker = "      - name: Review push without an open PR\n"
        self.assertIn(marker, workflow.read_text(), "Push review step is missing or renamed")
        step = workflow.read_text().split(marker, 1)[1]
        script = textwrap.dedent(step.split("\n      - name:", 1)[0].split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, bin_dir = root / "repo", root / "bin"
            repo.mkdir()
            bin_dir.mkdir()

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=repo, text=True).strip()

            git("init", "-q", "-b", "main")
            git("config", "user.name", "Review test")
            git("config", "user.email", "review-test@example.invalid")
            git("config", "commit.gpgsign", "false")
            source = repo / "source.rs"
            source.write_text("fn baseline() {}\n")
            git("add", ".")
            git("commit", "-qm", "baseline")
            baseline = git("rev-parse", "HEAD")
            source.write_text("fn changed() {}\n")
            git("commit", "-qam", "change")
            changed = git("rev-parse", "HEAD")
            git("update-ref", "refs/remotes/origin/main", baseline)
            git("checkout", "--detach", "-q", baseline)
            source.write_text("fn divergent() {}\n")
            git("commit", "-qam", "divergent change")
            divergent = git("rev-parse", "HEAD")
            for name, body in {
                "npm": "#!/bin/sh\nexit 0\n",
                "ocr": ('#!/bin/sh\n'
                        'case "$1" in review|scan)\n'
                        '  printf "%s\\n" "$1" > "$OCR_TEST_MODE"\n'
                        '  while [ "$#" -gt 1 ]; do\n'
                        '    if [ "$1" = --commit ] || [ "$1" = --from ]; then\n'
                        '      printf "%s\\n" "$1" "$2" > "$OCR_TEST_RANGE"; break\n'
                        '    fi\n'
                        '    shift\n'
                        '  done;;\n'
                        'esac\n'),
            }.items():
                path = bin_dir / name
                path.write_text(body)
                path.chmod(0o755)
            script = script.replace("/tmp/ocr-result.json", str(root / "result.json"))
            script = script.replace("/tmp/ocr-stderr.log", str(root / "stderr.log"))
            cases = [(baseline, changed, None), (changed, baseline, changed),
                     ("0" * 40, changed, None), ("f" * 40, changed, ""),
                     (changed, divergent, changed)]
            for before, head, expected_parent in cases:
                with self.subTest(before=before, head=head):
                    git("checkout", "--detach", "-q", head)
                    range_file = root / "range"
                    for output in (range_file, root / "mode", root / "result.json"):
                        output.unlink(missing_ok=True)
                    env = dict(os.environ, PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
                               BEFORE_SHA=before, HEAD_SHA=head, DEFAULT_BRANCH="main",
                               GITHUB_REF_NAME="feature", OCR_LLM_PROTOCOL="openai",
                               OCR_EXTRA_BODY="{}", OCR_BACKGROUND="test",
                               OCR_TEST_MODE=str(root / "mode"), OCR_TEST_RANGE=str(range_file))
                    process = subprocess.run(["bash", "-e"], input=script, cwd=repo, env=env,
                                             text=True, capture_output=True)
                    self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
                    self.assertTrue((root / "result.json").is_file())
                    self.assertEqual((root / "mode").read_text().strip(), "review")
                    self.assertEqual(git("rev-parse", "HEAD"), head)
                    flag, comparison = range_file.read_text().splitlines()
                    if expected_parent is None:
                        self.assertEqual((flag, comparison),
                                         ("--from", "origin/main" if before == "0" * 40 else before))
                    else:
                        self.assertEqual(flag, "--commit")
                        self.assertEqual(git("show", "-s", "--format=%P", comparison), expected_parent)
                        self.assertEqual(git("rev-parse", comparison+"^{tree}"), git("rev-parse", head+"^{tree}"))
                        self.assertIn("source.rs", git("diff-tree", "--root", "--no-commit-id", "--name-only", "-r", comparison))


if __name__ == "__main__":
    unittest.main()
