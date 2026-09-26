#!/usr/bin/env bash
# Compose docs/book into one PDF. Mermaid fences are rendered to PNG
# (mermaid.ink) and inlined. Source Markdown is left unchanged.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
OUT="$ROOT/bork-in-action.pdf"

python3 - "$ROOT" "$WORK" << 'PY'
import base64, pathlib, re, sys, urllib.request
root, work = map(pathlib.Path, sys.argv[1:])
order = [
    "00-przedmowa.md",
    "01-o-tej-ksiazce.md",
    "02-wprowadzenie.md",
    "03-filozofia.md",
    "04-skladnia.md",
    "05-typy.md",
    "06-funkcje.md",
    "07-przeplyw.md",
    "08-tablice.md",
    "09-regiony.md",
    "10-stringi.md",
    "11-bledy.md",
    "12-biblioteka-idiomy.md",
    "13-pipeline.md",
    "14-parser.md",
    "15-sema.md",
    "16-typeck.md",
    "17-pamiec-srodkowa.md",
    "18-codegen.md",
    "19-lsp.md",
    "20-mapa.md",
    "21-slad.md",
    "22-praca-z-repo.md",
    "23-cwiczenia.md",
    "A-sciagawka.md",
    "B-slowniczek.md",
    "C-niedokonczone.md",
]
fig = work / "fig"
fig.mkdir()
n = 0
fence = re.compile(
    r"(?:<!--\s*figura:\s*(.*?)\s*-->\n)?```mermaid\n(.*?)\n```",
    re.S,
)

def render(body: str) -> str:
    global n
    def repl(match: re.Match) -> str:
        global n
        n += 1
        caption = (match.group(1) or f"Diagram {n}").strip()
        src = match.group(2).strip() + "\n"
        encoded = base64.urlsafe_b64encode(src.encode()).decode().rstrip("=")
        url = "https://mermaid.ink/img/" + encoded + "?type=png&bgColor=white"
        dest = fig / f"diagram-{n}.png"
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "bork-book"})
            with urllib.request.urlopen(req, timeout=60) as resp:
                data = resp.read()
            if not data.startswith(b"\x89PNG"):
                raise RuntimeError("not png")
            dest.write_bytes(data)
            return f"![{caption}]({dest.as_posix()})"
        except Exception as err:
            print(f"mermaid {n} failed: {err}", file=sys.stderr)
            return "```text\n" + src + "```"
    return fence.sub(repl, body)

parts = []
for name in order:
    text = (root / name).read_text()
    parts.append(render(text))
    parts.append("\n\n")
(work / "book.md").write_text("".join(parts))
print(f"diagrams: {n}")
PY

REPO="$(cd "$ROOT/../.." && pwd)"
LOGO="$REPO/assets/bork-logo.png"
if [[ ! -f "$LOGO" ]]; then
  echo "missing logo: $LOGO" >&2
  exit 1
fi
printf '\\newcommand{\\borklogo}{%s}\n' "$LOGO" > "$WORK/logo-path.tex"

pandoc "$WORK/book.md" \
  --from markdown+pipe_tables+fenced_code_blocks+backtick_code_blocks \
  --syntax-definition "$ROOT/tools/bork.xml" \
  --lua-filter "$ROOT/tools/callouts.lua" \
  --pdf-engine=xelatex \
  --toc \
  --number-sections \
  --top-level-division=chapter \
  -V documentclass=report \
  -V lang=pl \
  -V title="Bork in Action" \
  -V subtitle="Język, kompilator i mapa repozytorium" \
  -V author="Na podstawie kodu Bork" \
  -V date="2026" \
  -V mainfont="DejaVu Serif" \
  -V sansfont="DejaVu Sans" \
  -V monofont="DejaVu Sans Mono" \
  -V fontsize=11pt \
  -V geometry:margin=2.3cm \
  -V colorlinks=true \
  -V linkcolor=NavyBlue \
  -H "$WORK/logo-path.tex" \
  -H "$ROOT/tools/header.tex" \
  -o "$OUT"

echo "wrote $OUT"
pdfinfo "$OUT" | sed -n '1,12p'
