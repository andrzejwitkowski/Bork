"""Sprawdza kompletność dokumentu i, opcjonalnie, ścieżki w snapshotcie Borka."""
from pathlib import Path
from collections import Counter
import hashlib
import json
import re
import shutil
import sys
from pypdf import PdfReader

ROOT=Path(__file__).resolve().parent
SHA='56f4910a24b689087fe2dee30247651b7b432268'
source=Path(sys.argv[1]) if len(sys.argv)>1 else None
md=(ROOT/'podrecznik-sklad.md').read_text()
pdf=ROOT/'Bork-budowa-kompilatora.pdf'
reader=PdfReader(pdf,strict=True)
text='\n'.join(page.extract_text() for page in reader.pages)
chapters=re.findall(r'^# (\d+)\. (.+)',md,re.M)
assert [int(n) for n,_ in chapters]==list(range(1,25))
for n,title in chapters:
    assert title in text, (n,title)
assert '[[diagram:' not in text and '[[indeks]]' not in text
assert '\ufffd' not in text and '\u25a0' not in text
assert all(char in text for char in set(md).intersection('ąćęłńóśźżĄĆĘŁŃÓŚŹŻ'))
definitions=json.loads((ROOT/'rysunki.json').read_text())
assert len(definitions)==46
assert len(re.findall(r'Rysunek \d+\.\d+\.',text))==46
for key in definitions:
    assert (ROOT/'rysunki'/f'{key}.svg').stat().st_size>200

prefix=f'https://github.com/andrzejwitkowski/Bork/blob/{SHA}/'
links=re.findall(re.escape(prefix)+r'([^\s)#]+)',md)
links=sorted(set(links))
fixtures=set(re.findall(r'programs/(?:build|build_fail|check|check_fail)/[\w/-]+\.bork',md))
if source:
    for path in links:
        assert (source/path).is_file(),path
    for path in fixtures:
        src=source/path
        assert src.is_file(),path
        dest=ROOT/'przyklady'/path
        dest.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(src,dest)
        assert src.read_bytes()==dest.read_bytes()

uris=0;internal=0
for page in reader.pages:
    assert len(page.extract_text())>100,'Pusta strona'
    for annot in page.get('/Annots',[]):
        a=annot.get_object()
        if a.get('/A',{}).get('/URI'):uris+=1
        if a.get('/Dest'):internal+=1
assert uris>=100 and internal>=100,(uris,internal)
index=json.loads((ROOT/'indeks.json').read_text())
assert all(1<=n<=24 for nums in index.values() for n in nums)
stats={
    'commit':SHA,'branch':'main','prepared':'2026-11-02',
    'pages':len(reader.pages),'chapters':24,'appendices':2,
    'figures':len(definitions),'bork_code_blocks':md.count('```bork'),
    'all_code_blocks':len(re.findall(r'^```\w+',md,re.M)),
    'source_words':len(md.split()),'index_terms':len(index),
    'external_pdf_links':uris,'internal_pdf_links':internal,
    'distinct_source_paths':len(links),'included_programs':len(fixtures),
    'pdf_sha256':hashlib.sha256(pdf.read_bytes()).hexdigest(),
    'pdf_bytes':pdf.stat().st_size,
    'corpus_test_command':'cargo test --offline --no-default-features --features codegen --test programs -- --nocapture',
    'corpus_test_result':'3 passed; 0 failed; 25.04s',
    'corpus_fixture_counts':{'check':50,'check_fail':128,'build':189,'build_fail':10},
    'environment':'LLVM 23; GCC used only as clang-named final linker; snapshot-built bork',
    'verification_scope':'Corpus fixtures executed. Explanatory fragments and schematic IR are not independent runnable programs.',
    'visual_review':'Rendered page samples, Polish glyphs, diagram placement, tables and navigation checked separately.'
}
(ROOT/'weryfikacja.json').write_text(json.dumps(stats,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(stats,ensure_ascii=False,indent=2))
