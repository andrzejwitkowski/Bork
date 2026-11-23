"""Fail a CI check for serious findings or incomplete OCR coverage."""

import json
import os
from pathlib import Path
import sys


def strict_errors(result):
    manifest = result["manifest"]
    coverage = manifest["coverage"]
    status = result["status"]
    selected = {item["item_id"] for item in coverage["selected"] or []}
    completed = {item["item_id"] for item in coverage["completed"] or []}
    completed.update(item["item_id"] for item in coverage["reused"] or [])
    errors = []
    if status not in ("complete", "skipped") or manifest["terminal_state"] != status:
        errors.append("review did not complete")
    if coverage.get("failed") or coverage.get("waived") or selected != completed:
        errors.append("some selected files were not reviewed")
    if status == "skipped" and selected:
        errors.append("review skipped selected files")
    if any(item["exclude_reason"] == "too_large" for item in coverage["excluded"] or []):
        errors.append("a changed file exceeded the review size limit")
    if (result.get("summary") or {}).get("budget_exceeded") or any(
        warning.get("type") in ("review_limit_reached", "token_budget_reached")
        for warning in result.get("warnings") or []
    ):
        errors.append("review hit a token or round limit")
    blocking = sum(
        comment.get("severity", "").lower() in ("high", "critical")
        for comment in result.get("comments") or []
    )
    if blocking:
        errors.append(f"{blocking} high/critical finding(s)")
    return errors


def main():
    try:
        result = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
        errors = strict_errors(result)
    except (OSError, ValueError, KeyError, TypeError, AttributeError, IndexError):
        errors = ["missing or invalid OCR result/coverage manifest"]
    message = "Strict review: " + ("; ".join(errors) if errors else "passed")
    print(message)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write(message + "\n\nSee PR comments or the OCR JSON artifact for findings.\n")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
