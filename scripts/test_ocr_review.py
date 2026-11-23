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


class PushRangeTests(unittest.TestCase):
    def test_pushes_do_not_turn_rewinds_into_empty_reviews(self):
        workflow = GATE.parents[1] / ".github/workflows/ocr-review.yml"
        step = workflow.read_text().split("      - name: Review push without an open PR\n", 1)[1]
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
            for name, body in {
                "npm": "#!/bin/sh\nexit 0\n",
                "ocr": '#!/bin/sh\ncase "$1" in review|scan) printf "%s\\n" "$1" > "$OCR_TEST_MODE";; esac\n',
            }.items():
                path = bin_dir / name
                path.write_text(body)
                path.chmod(0o755)
            script = script.replace("/tmp/ocr-result.json", str(root / "result.json"))
            script = script.replace("/tmp/ocr-stderr.log", str(root / "stderr.log"))
            cases = [(baseline, changed, "review"), (changed, baseline, "scan"),
                     ("0" * 40, changed, "review"), ("f" * 40, changed, "scan")]
            for before, head, expected in cases:
                with self.subTest(before=before, head=head):
                    git("checkout", "--detach", "-q", head)
                    env = dict(os.environ, PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
                               BEFORE_SHA=before, HEAD_SHA=head, DEFAULT_BRANCH="main",
                               GITHUB_REF_NAME="feature", OCR_LLM_PROTOCOL="openai",
                               OCR_EXTRA_BODY="{}", OCR_BACKGROUND="test",
                               OCR_TEST_MODE=str(root / "mode"))
                    subprocess.run(["bash", "-e"], input=script, cwd=repo, env=env,
                                   text=True, capture_output=True, check=True)
                    self.assertEqual((root / "mode").read_text().strip(), expected)


if __name__ == "__main__":
    unittest.main()
