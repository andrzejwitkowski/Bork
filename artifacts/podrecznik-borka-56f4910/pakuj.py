"""Pakuje materiały, zapisując archiwum poza katalogiem wejściowym."""
from pathlib import Path
import zipfile

root=Path(__file__).resolve().parent
target=root.parent/'Bork-podrecznik-56f4910-materialy.zip'
assert not target.is_relative_to(root)
files=[p for p in sorted(root.rglob('*')) if p.is_file()
       and not set(p.relative_to(root).parts).intersection({'__pycache__','kontrola'})
       and p.suffix not in {'.zip','.part'}]
with zipfile.ZipFile(target,'w',zipfile.ZIP_DEFLATED,compresslevel=6) as archive:
    for path in files:
        archive.write(path,Path(root.name)/path.relative_to(root))
with zipfile.ZipFile(target) as archive:
    assert archive.testzip() is None
    assert len(archive.infolist())==len(files)
print(f'{target}\n{len(files)} plików, {target.stat().st_size} bajtów')
