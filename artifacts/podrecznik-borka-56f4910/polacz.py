"""Łączy treść rozdziałów i zastępuje wskazówki do ilustracji gotowymi podpisami."""
from pathlib import Path
import json
import re

ROOT=Path(__file__).resolve().parent
parts=['wstep.md','rozdzialy/02-08.md','rozdzialy/09-14.md',
       'rozdzialy/15-20.md','zakonczenie.md']
captions=json.loads((ROOT/'podpisy.json').read_text())
text='\n\n'.join((ROOT/p).read_text().strip() for p in parts)+'\n'
def caption(m):
    key,title=m.group(1),m.group(2)
    return f'[[diagram:{key}|{title}|{captions[key]}]]'
text=re.sub(r'\[\[diagram:([^|]+)\|([^|]+)\|.*?\]\]',caption,text,flags=re.S)
# Rysunki stoją przy objaśnianym mechanizmie, zamiast tworzyć galerię na końcu rozdziału.
placements={
    'front-01':'## Wiązanie nie jest obiektem',
    'front-02':'## Wyrażenia i przepływ sterowania',
    'front-03':'## Priorytety zapisane jako warstwy gramatyki',
    'front-04':'## Pułapka i ćwiczenie',
    'front-05':'## HIR utrwala rozstrzygnięcia semantyczne',
    'front-06':'## Orkiestracja i granice reprezentacji',
    'front-07':'## Unifikacja, ograniczenia i transakcje',
    'front-08':'## Sprzeczność jest błędem, nie przeciążeniem',
    'front-09':'## Dopasowanie sprawdza strukturę i sposób dostępu',
    'front-10':'Liczby całkowite i napisy są modelowane',
    'front-11':'## Własność i plan pamięci muszą widzieć konkretne typy',
    'front-12':'Istnieje ograniczenie związane z odtwarzaniem argumentów',
    'front-13':'## Solver zobowiązań pracuje do zatrzymania postępu',
    'front-14':'## Rozstrzyganie metody ma osobne pułapki',
    'back-15a':'## Co naprawdę trafia do LLVM',
    'back-15b':'## Granice i kontrola zgodności',
    'back-16a':'## Leniwe operatory są sterowaniem',
    'back-16b':'## Ograniczenia i ćwiczenie',
    'back-17a':'## Ślad warunkowej obserwacji',
    'back-17b':'## Uchwyt lokalny, wynik gałęzi i pole obiektu',
    'back-18a':'## Cel natywny i rzeczywiste optymalizacje',
    'back-18b':'## Archiwum runtime i linkowanie',
    'back-19a':'## Przykład: pole osiągalne przez Ref',
    'back-19b':'## Ograniczenia i ćwiczenie',
    'back-20a':'## Korpus jako zestaw kontraktów',
    'back-20b':'## Ćwiczenie z odpowiedzią',
}
chapters=re.split(r'(?=^# )',text,flags=re.M)
for i,chapter in enumerate(chapters):
    for key,anchor in placements.items():
        marker=re.search(r'\[\[diagram:'+re.escape(key)+r'\|.*?\]\]',chapter,re.S)
        if not marker:continue
        diagram=marker.group()
        chapter=chapter.replace(diagram,'')
        assert anchor in chapter,(key,anchor)
        chapter=chapter.replace(anchor,diagram+'\n\n'+anchor,1)
    chapters[i]=chapter
text=''.join(chapters)
# Oczekiwania korpusu opisuje końcowy raport ze wspólnego uruchomienia.
text=text.replace('nie jest wynikiem\nnowego uruchomienia wykonanego podczas pisania tego podręcznika.',
    'jest komentarzem dla kompilatora. Weryfikację korpusu opisuje dodatek A.')
text=text.replace('Komentarze są oczekiwaniami istniejącego testu. Nie zastępują raportu\nz nowego uruchomienia.',
    'Komentarze są oczekiwaniami istniejącego testu. Weryfikację całego korpusu\nopisuje dodatek A.')
(ROOT/'podrecznik-sklad.md').write_text(text)
# Wersja Markdown do zwykłego czytania i dalszej redakcji.
def image(m):
    key,title,cap=m.groups()
    return f'![{title}](rysunki/{key}.svg)\n\n*{title}. {cap}*'
readable=re.sub(r'\[\[diagram:([^|]+)\|([^|]+)\|(.+?)\]\]',image,text,flags=re.S)
terms=json.loads((ROOT/'indeks.json').read_text())
readable=readable.replace('[[indeks]]','\n'.join(
    f'- **{term}**: rozdz. '+', '.join(map(str,chapters))
    for term,chapters in sorted(terms.items(),key=lambda x:x[0].lower())))
(ROOT/'podrecznik.md').write_text(readable)
print(f'{len(text.split())} słów, {len(re.findall(r"^# ",text,re.M))} rozdziałów i dodatków')
