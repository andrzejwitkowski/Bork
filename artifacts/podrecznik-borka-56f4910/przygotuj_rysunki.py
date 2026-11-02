"""Definicje merytorycznych schematów. Wynik: rysunki.json i podpisy.json."""
from pathlib import Path
import json
import re
import textwrap

ROOT=Path(__file__).resolve().parent
figures={}
captions={}
def fig(key,caption,body):
    figures[key]=body
    captions[key]=caption

fig('root-01','Numery wskazują rozdziały. Studium przypadku łączy wszystkie warstwy.',r'''
rankdir=LR;
a[label="2–8\nJęzyk i reprezentacje"]; b[label="9–14\nWłasność i pamięć"];
c[label="15–20\nKod natywny i narzędzia"]; d[label="21\nJeden pełny przebieg",fillcolor="#d5ebe6"];
a->b->c; {a b c}->d;
''')
fig('root-02','Analiza własności odwiedza AST, korzystając z informacji wyprowadzonych przez typeck.',r'''
src[label="Tekst źródłowy"]; ast[label="AST + prelude"]; tc[label="Typeck"];
hir[label="Typowany HIR"]; sema[label="Sema na AST\n+ typy i member resolutions"];
ok[label="Brak diagnostyk?",shape=diamond]; mem[label="Hoist → escape\n→ MemoryPlan → stamp"];
src->ast->tc; tc->hir; ast->sema[style=dashed]; tc->sema;
hir->ok; sema->ok; ok->mem[label="tak"];
{rank=same;hir;sema}
''')
fig('front-01','Przerywana strzałka oznacza pożyczenie odbiorcy; wynik metody jest liczbą.',r'''
rankdir=LR;
a[label="class Point\nx: i32, y: i32"];b[label="Point(3, 4)"];
p[label="p: Point\nx=3, y=4"];m[label="sum(this: &Point)\nx + y"];
r[label="main: return 7",fillcolor="#d5ebe6"];
a->b->p;p->m[style=dashed,label="borrow"];m->r[label="i32"];
''')
fig('front-02','Wartość, pożyczka, zarządzany uchwyt i opcja mają różne obowiązki.',r'''
root[label="Rodzaj dostępu",fillcolor="#d5ebe6"];
v[label="T\nwłasność wartości"];b[label="&T\npożyczony widok"];
r[label="Ref<T>\nzarządzany uchwyt"];o[label="Option<T> = T?\nSome(T) / None"];
root->{v b r o}; b->v[style=dashed,label="nie przedłuża życia"];
r->target[label="cel może wygasnąć"]; target[label="cel w arenie"];
''')
fig('front-03','Zamiana LF na CR ma długość jednego bajtu i zachowuje offsety w oryginalnym tekście.',r'''
rankdir=LR;
a[shape=record,label="Źródło|call( LF|  1, LF|  2 LF|)"];
b[shape=record,label="Normalizacja|call( CR|  1, CR|  2 CR|)"];
c[label="Tokeny\ncall ( 1 , 2 )"];
d[label="LF wewnątrz\nzagnieżdżonego bloku\npozostaje separatorem",color="#b15f36"];
a->b[label="1 bajt → 1 bajt"];b->c[label="lexer pomija CR"];a->d[style=dashed];
''')
fig('front-04','Dwa drzewa mają odrębne przestrzenie identyfikatorów. Mnożenie wiąże mocniej niż dodawanie.',r'''
subgraph cluster_ast {label="AST: 1 + 2 * 3";color="#cbdadb";
a[label="Add"]; b[label="1"];c[label="Mul"];d[label="2"];e[label="3"];
a->{b c};c->{d e};}
subgraph cluster_hir {label="HIR: inne NodeId";color="#cbdadb";
h[label="Binary Add : i32"];i[label="Int 1 : i32"];j[label="Binary Mul : i32"];
h->{i j};}
idx[label="Span → AstId\nindeks AST"];
a->h[style=dashed,label="lowering"];idx->a[style=dashed];
''')
fig('front-05','Pierwsze trafienie kończy wyszukiwanie zwykłego x. Zapis this.x jawnie wskazuje pole.',r'''
rankdir=LR;
x[label="x w metodzie"];local[label="Lokalne zasięgi\ni parametry"];
field[label="Pole klasy\nprzez this"];
global[label="Nazwy globalne"];
explicit[label="this.x"];idx[label="Indeks pola x"];
x->local;local->field[label="brak trafienia"];field->global[label="brak trafienia"];
local->stop[label="parametr x istnieje"];stop[label="Użyj parametru",fillcolor="#d5ebe6"];
explicit->idx;
''')
fig('front-06','Każdy identyfikator odpowiada na inne pytanie; MemberResolutions przenosi rozstrzygnięcie członka do analiz AST.',r'''
p[label="p.sum()",fillcolor="#d5ebe6"];
s[label="Span\ngdzie w źródle?"];a[label="AstId\nktóry węzeł AST?"];
d[label="DefId metody\nktóra definicja?"];t[label="TyId\njaki typ?"];
n[label="NodeId\nktóry węzeł HIR?"];
p->{s a d t n}; m[label="MemberResolutions\nAST ↔ znaczenie członka"];a->m;d->m;
''')
fig('front-07','Późniejsze użycie dostarcza ograniczenia, zanim zostanie wybrany typ domyślny.',r'''
rankdir=LR;
a[label="val x = 1\nα: Int"];b[label="takes_u8(x)\nα = u8"];
c[label="Koniec ciała\nx: u8 w HIR",fillcolor="#d5ebe6"];
d[label="Literał f\nβ: Float"];e[label="Użycie z f32\nβ = f32"];
a->b->c;d->e;
''')
fig('front-08','Transakcja unifikacji nie pozostawia częściowego związania po odrzuceniu kandydata.',r'''
rankdir=LR;
a[label="Stan początkowy\nα nierozwiązane"];
b[label="Snapshot"];
c[label="Próba\n(α) → bool\n(i32) → String"];
d[label="α := i32\npotem konflikt wyniku",color="#b15f36"];
e[label="Rollback\nα nierozwiązane",fillcolor="#d5ebe6"];
a->b->c->d->e;
''')
fig('front-09','Zagnieżdżona opcja ma trzy różne kategorie wartości. Nieobecność Ref należy do innego protokołu.',r'''
o[label="Option<Option<i32>>"];
n[label="None"];s[label="Some(wewnętrzna opcja)"];
sn[label="Some(None)"];ss[label="Some(Some(liczba))"];
o->{n s};s->{sn ss};
r[label="Ref<Node>\nNone lub obecny cel",color="#b15f36"];
{rank=same;o;r}
''')
fig('front-10','Ramiona None i Some(true) nie pokrywają wartości Some(false).',r'''
a[label="when: Option<bool>\nNone, Some(true)"];
n[label="Wariant None\npokryty",fillcolor="#d5ebe6"];
s[label="Wariant Some\npole bool"];
t[label="true: pokryte",fillcolor="#d5ebe6"];
f[label="false: brak\nświadek Some(false)",color="#b15f36",fillcolor="#faeee7"];
a->{n s};s->{t f};
''')
fig('front-11','Mapa instancji współdzieli powtórne żądanie tego samego klucza funkcja–argumenty typowe.',r'''
m[label="Kolejka: main"];
i[label="id<i32>"];s[label="id<String>"];p[label="id<Point>"];
m->{i s p};again[label="Drugie id<i32>"];
again->i[label="trafienie w mapie",style=dashed];
k[label="Klucz:\nfunkcja + typy argumentów",shape=note];k->i[style=dashed];
''')
fig('front-12','Specjalizacja wymaga odświeżenia metadanych, ponieważ konkretny typ zmienia potrzeby pamięciowe.',r'''
rankdir=LR;
a[label="Generyczny HIR\nParam(0)"];b[label="Podstawienie\nT = String"];
c[label="Konkretny HIR\nString"];
d[label="Raport instancji\nnowe NodeId"];
e[label="Escape + plan\ndla instancji",fillcolor="#d5ebe6"];
a->b->c->d->e;
''')
fig('front-13','Wybór implementacji odbywa się podczas kompilacji. W wykonaniu pozostaje bezpośrednie wywołanie.',r'''
rankdir=LR;
a[label="say<T: Show>"];b[label="say(&c)\nCircle: Show"];
c[label="impl Show\nfor Circle"];d[label="say<Circle>\nbezpośredni callee"];
e[label="Runtime\ncall Circle.show",fillcolor="#d5ebe6"];
a->b->c->d->e;
''')
fig('front-14','Unifikujące się nagłówki konfliktują. Różne, nieunifikujące się argumenty traitu mogą je rozdzielać.',r'''
a[label="impl<T> Show for Box<T>"];
b[label="impl Show for Box<Circle>"];
w[label="T = Circle\nKONFLIKT",color="#b15f36",fillcolor="#faeee7"];
a->w;b->w;
x[label="Conv<i32> for Box<Circle>"];y[label="Conv<bool> for Box<Circle>"];
ok[label="i32 ≠ bool\nrozłączne",fillcolor="#d5ebe6"];x->ok;y->ok;
''')
fig('memory-09','Do stanu po if wchodzą tylko ścieżki, które do niego docierają.',r'''
subgraph cluster_a {label="Gałąź kończy się return";color="#cbdadb";
a[label="x dostępne"];b[label="move x → return"];
c[label="bez move"];d[label="Dalsze użycie x: OK",fillcolor="#d5ebe6"];
a->{b c};c->d;}
subgraph cluster_b {label="Obie gałęzie docierają dalej";color="#cbdadb";
e[label="x dostępne"];f[label="move x"];g[label="bez move"];
h[label="x być może przeniesione\nużycie odrzucone",color="#b15f36"];
e->{f g};f->h;g->h;}
''')
fig('memory-10','Obie pożyczki wskazują ten sam bufor. Ich zakresy mieszczą się w życiu właściciela.',r'''
rankdir=LR;
subgraph cluster_main {label="main: właściciel";color="#087e83";
buf[shape=record,label="Bufor|1|2|3"];
subgraph cluster_outer {label="outer: pożyczka p";color="#80b6b0";
p[label="p: &[i32; 3]"];
subgraph cluster_inner {label="inner";color="#cbdadb";q[label="buf: &[i32; 3]"];}}}
p->buf[style=dashed];q->buf[style=dashed];
''')
fig('memory-11','Wybór miejsca, sprawdzenie bezpieczeństwa i klasyfikacja obiektu są odrębnymi krokami.',r'''
rankdir=LR;
a[label="Typowany HIR"];b[label="Hoist\ncel alokacji"];
c[label="Escape\nważność przepływu"];d[label="MemoryPlan\nklasa + domena"];
e[label="LLVM\nwywołania runtime"];
a->b->c->d->e;
''')
fig('memory-12','Domena propaguje się przez zależności do punktu stałego; zwykłe zagnieżdżenie i Ref są różnymi krawędziami.',r'''
rankdir=LR;
a[label="A\nwynik funkcji\nProgram",fillcolor="#d5ebe6"];
b[label="B\nobiekt zagnieżdżony\nProgram",fillcolor="#d5ebe6"];
c[label="C\nProgram po propagacji"];
d[label="D\nProgram po kolejnym kroku"];
a->b[label="posiada"];b->c[label="Ref"];c->d[label="Ref"];
''')
fig('memory-13','Schemat ilustruje klasy alokacji i kolejność poszukiwania miejsca; nie pokazuje rzeczywistej skali zajętości.',r'''
rankdir=LR;
q[label="Żądanie rozmiaru\ni wyrównania"];
free[shape=record,label="8 klas wolnych bloków|16|32|64|128|256|512|1024|2048"];
p1[shape=record,label="Strona 4096 B|zajęte|padding|zajęte|wolny ogon"];
p2[shape=record,label="Starsze strony|zajęte|wolne końcówki"];
new[label="Nowa strona"];
big[label="Duży blok\nżądanie > 2048 B",color="#b15f36"];
q->free[label="małe"];free->p1[label="brak"];p1->p2[label="brak"];p2->new[label="brak"];q->big[label="duże"];
''')
fig('memory-14','Po śmierci payloadu kontrola zachowuje tożsamość potrzebną słabym uchwytom.',r'''
rankdir=LR;
a[label="Alive\nControl id=41"];
b[label="Destroying\nwykonaj dropy"];
c[label="Dead\nControl id=41\nbez payloadu",color="#b15f36"];
a->b->c;
w[label="Słaby uchwyt\ndo id=41"];w->c[style=dashed];
p[label="Strony: recykling"];
n[label="Nowa arena\nid=42",fillcolor="#d5ebe6"];
b->p->n;c->end[label="ostatni weak znika"];end[label="Zwolnienie kontroli"];
''')
fig('memory-15','Zachowany child może żyć po utracie parent. Słaba krawędź nie odtwarza życia payloadu rodzica.',r'''
rankdir=LR;
s[label="saved\nstarsza arena"];
c[label="C: child\npayload żywy",fillcolor="#d5ebe6"];
p[label="P: parent\npayload wygasł\nkontrola pozostała",color="#b15f36"];
s->c[label="Strong"];c->p[label="Weak",style=dashed];
g1[label="guard saved: obecny"];g2[label="guard parent: None"];
c->g1;p->g2;
''')
fig('memory-16','Niepowodzenie kolejnego guarda zwalnia wcześniej zdobyte obserwacje.',r'''
rankdir=LR;
a[label="observe r → a"];b[label="observe a.next → b"];body[label="Ciało"];
ra[label="release a"];rb[label="release b\nrelease a"];
els[label="else"];exit[label="Cel sterowania"];
a->b[label="tak"];a->els[label="nie"];
b->body[label="tak"];b->ra[label="nie"];ra->els;
body->rb->exit;
''')
fig('memory-17','Poprawne UTF-8 zostaje skopiowane do niezależnego String. Błąd zakresu i błąd kodowania mają różne wyniki.',r'''
a[shape=record,label="Bufor [u8; N]|przed|start … start+count|po"];
b[label="Kontrola granic"];
c[label="Walidacja UTF-8"];
abort[label="abort\nzły zakres",color="#b15f36"];
none[label="None\nzłe UTF-8",color="#b15f36"];
dst[shape=record,label="Nowy String w sink|własne bajty UTF-8",fillcolor="#d5ebe6"];
a->b;b->abort[label="błąd"];b->c[label="OK"];c->none[label="błąd"];c->dst[label="OK: kopia"];
''')
fig('memory-18','Slice zmienia adres i długość deskryptora, ale nadal korzysta z pamięci tego samego właściciela.',r'''
rankdir=LR;
a[shape=record,label="a: [i32; 3]|ptr|len=3"];
b[shape=record,label="a[0..2]|ptr|len=2"];
c[shape=record,label="a[1..3]|ptr + sizeof(i32)|len=2"];
buf[shape=record,label="Jeden bufor|<x>element 0|<y>element 1|element 2"];
a->buf:x;b->buf:x[style=dashed];c->buf:y[style=dashed];
''')
fig('memory-19','Nagłówek utrzymuje bieżący bufor. Wzrost może udać się w miejscu albo wymagać nowego przydziału.',r'''
rankdir=LR;
h[shape=record,label="Nagłówek H|len|cap|buf"];
b4[shape=record,label="cap=4|0|1|2|3"];
b8[shape=record,label="cap=8|0 … 4|wolne sloty"];
b16[shape=record,label="cap=16|0 … 9|wolne sloty"];
h->b4[label="przed wzrostem",style=dashed];h->b8[style=dashed];h->b16[label="aktualnie"];
b4->b8->b16[label="grow / kopia"];
''')
fig('memory-20','Promocja kontenera ponownie klasyfikuje krawędzie względem nowej areny; obiekty docelowe nie zmieniają miejsca.',r'''
subgraph cluster_before {label="Przed promocją";color="#cbdadb";
t[label="tmp: Vec<Ref>\narena id=30"];
n1[label="N1\nid=20"];n2[label="N2\nid=21"];
t->n1[label="Weak",style=dashed];t->n2[label="Weak",style=dashed];}
s[label="saved: kopia Vec\narena id=10",fillcolor="#d5ebe6"];
t->s[label="promote",color="#b15f36"];
s->n1[label="Strong"];s->n2[label="Strong"];
''')
fig('back-15a','Kolejne etapy mogą zgłaszać diagnostykę; poprawny HIR nie gwarantuje jeszcze gotowości backendu.',r'''
rankdir=LR;
a[label="frontend::check"];b[label="HIR + raport"];
c[label="Monomorfizacja"];d[label="Gate"];
e[label="LLVM"];
err[label="Diagnostyka",color="#b15f36"];
a->b->c->d->e;
a->err[style=dashed];c->err[style=dashed];d->err[style=dashed];
''')
fig('back-15b','Deskryptor zawiera odwołania do danych; nie należy mylić go z samym payloadem.',r'''
rankdir=LR;
s[shape=record,label="String|<p>ptr|len"];
o[shape=record,label="Obiekt|<c>control|<p>object"];
r[shape=record,label="Ref|<c>control|<p>object|kind"];
bytes[label="Bajty UTF-8"];
control[label="Sterowanie areny"];
payload[label="Payload obiektu"];
s:p->bytes;o:c->control;r:c->control;o:p->payload;r:p->payload;
''')
fig('back-16a','Phi wybiera wartość z rzeczywistego poprzednika bloku merge, a nie oblicza ponownie warunku.',r'''
c[label="flag == 1 ?",shape=diamond];
t[label="then\n40"];f[label="else\n7"];
m[label="merge\nφ [40, then], [7, else]",fillcolor="#d5ebe6"];
r[label="v + 2 → return"];
c->t[label="tak"];c->f[label="nie"];t->m[label="then"];f->m[label="else"];m->r;
''')
fig('back-16b','Parametry techniczne przekazują kontekst pamięci, którego nie widać w sygnaturze źródłowej.',r'''
rankdir=LR;
c[label="Wywołujący"];
f[label="Zwykła funkcja\n(parent, argumenty)"];
g[label="Zwraca String\n(parent, sink, argumenty)"];
sink[label="Arena wyniku\nbajty String",fillcolor="#d5ebe6"];
c->f[label="parent"];c->g[label="parent + sink"];g->sink[label="alokuj wynik"];
''')
fig('back-17a','Każda krawędź sprząta zakresy, które rzeczywiście opuszcza. Wiersz dotyczy schematu sterowania.',r'''
rankdir=LR;
subgraph cluster_scope {label="Aktywne zakresy";color="#cbdadb";
f[label="Funkcja"];l[label="Pętla"];i[label="Wnętrze bloku"];f->l->i;}
ret[label="return\ncleanup do wyjścia funkcji"];
br[label="break\ncleanup do wyjścia pętli"];
co[label="continue\ncleanup do latch / reset"];
i->{ret br co};
''')
fig('back-17b','Przy częściowym sukcesie łańcucha sprzątane są tylko guardy, które zostały otwarte.',r'''
rankdir=LR;
a[label="Guard 1"];b[label="Guard 2"];c[label="Guard 3"];
body[label="Ciało → wynik w sink"];
cl1[label="Zwolnij guard 1"];
cl2[label="Zwolnij guard 2 i 1"];
els[label="else"];
a->b[label="sukces"];b->c[label="sukces"];c->body[label="sukces"];
a->els[label="brak"];b->cl1[label="brak"];c->cl2[label="brak"];
cl1->els;cl2->els;
''')
fig('back-18a','Linker łączy kod wygenerowany przez LLVM z archiwum runtime i bibliotekami systemowymi.',r'''
rankdir=LR;
a[label="Moduł LLVM"];b[label="Pipeline LLVM"];
o[label="Tymczasowy plik .o"];l[label="clang\nlinkowanie"];
r[label="libbork_runtime.a"];sys[label="Biblioteki systemowe"];
exe[label="Program natywny",fillcolor="#d5ebe6"];
a->b->o->l->exe;r->l;sys->l;
''')
fig('back-18b','Profil narzędzia Rust, optymalizacja programu Borka i profil runtime to odrębne decyzje.',r'''
a[label="Profil Cargo\ndev / release\nkompilator Borka"];
b[label="bork build\nIR: default<O3> + passes\ntarget: Default"];
c[label="bork build -g\nIR: default<O0>\ntarget: None"];
r[label="Runtime zwykły\nrustc opt-level=2"];
d[label="Runtime debug\nopt-level=0, debuginfo=2"];
a->b[style=dashed,label="uruchamia"];a->c[style=dashed];b->r;c->d;
''')
fig('back-19a','Każdy widok służy innemu pytaniu. Wydruk aren nie jest obrazem całej pamięci procesu.',r'''
root[label="Co chcę ustalić?",fillcolor="#d5ebe6"];
a[label="ArenaReport\nregiony i własność"];
b[label="MemoryPlan\ngdzie ma żyć obiekt?"];
c[label="LLVM przed optymalizacją\njakie instrukcje emitowano?"];
d[label="Graf runtime\nco jest teraz żywe?"];
root->{a b c d};
''')
fig('back-19b','Informacja o źródle jest przenoszona do metadanych i do mechanizmów inspekcji programu.',r'''
rankdir=LR;
s[label="Span źródła"];
dw[label="Lokalizacja DWARF"];
md[label="Metadane bork.span"];
rt[label="Hooki i rejestr\ndebug runtime"];
dbg[label="GDB / LLDB\nźródło + areny + Ref"];
s->dw->dbg;s->md->rt->dbg;
''')
fig('back-20a','Publikacja wyników musi odpowiadać aktualnemu dokumentowi; zamknięcie usuwa przechowany stan.',r'''
rankdir=LR;
open[label="didOpen / didChange"];
text[label="Tekst + epoka"];
check[label="check → gate"];
analysis[label="Analysis\naktualna epoka?"];
out[label="Diagnostyki\nhover / dump"];
close[label="didClose → usuń stan",color="#b15f36"];
open->text->check->analysis->out;close->analysis[style=dashed,label="unieważnia"];
''')
fig('back-20b','Testy powinny utrwalać kontrakty na granicach reprezentacji, a nie tylko ostatni wynik programu.',r'''
rankdir=LR;
a[label="Źródło\nparser"];
b[label="Typy + HIR\nwłasność"];
c[label="Mono\nplan pamięci"];
d[label="Backend\nABI + runtime"];
e[label="Program\ndebugger"];
a->b->c->d->e;
t1[label="Testy frontendowe"];t2[label="Golden planów"];
t3[label="Build / run + debug"];
t1->b[style=dashed];t2->c[style=dashed];t3->d[style=dashed];t3->e[style=dashed];
''')
fig('root-03','Podobna składnia zostaje rozdzielona po odnalezieniu deklaracji.',r'''
ast[label="AST: Call"];
classdef[label="Point jest klasą"];
funcdef[label="sum jest funkcją"];
obj[label="HIR: ObjectConstruct\npola 0, 1, 2"];
call[label="HIR: Call\ncallee + argumenty"];
ast->{classdef funcdef};classdef->obj;funcdef->call;
''')
fig('root-04','Region wiązania, domena planu i adres runtime są powiązane, ale nie są jednym opisem.',r'''
rankdir=LR;
a[label="Wiązanie p\nLocal w main"];
b[label="Plan Point\nDynamic / Program\nkonserwatywnie przez call"];
c[shape=record,label="Uchwyt runtime|control|object"];
d[label="Payload Point\nname, x, y"];
a->b[label="planowanie"];b->c[label="realizacja"];c->d;
''')
fig('root-05','Wymagania dotyczą tej samej niewiadomej. Nie da się ich spełnić przez wybór dwóch typów naraz.',r'''
a[label="Literał → α"];
u[label="Użycie 1\nα = u8"];
f[label="Użycie 2\nα = f32"];
err[label="u8 ≠ f32\ndiagnoza konfliktu",color="#b15f36",fillcolor="#faeee7"];
a->{u f};u->err;f->err;
''')
fig('root-06','Długi ciąg dopisań przechodzi głównie krótką ścieżką; wzrost wiąże się z dodatkową pracą.',r'''
push[label="push(element)"];
q[label="len < cap?",shape=diamond];
fast[label="Zapis w wolny slot"];
grow[label="Większa pojemność\nalokacja / powiększenie"];
copy[label="Obsłuż istniejące elementy\nzaktualizuj bufor"];
inc[label="len := len + 1",fillcolor="#d5ebe6"];
push->q;q->fast[label="tak"];q->grow[label="nie"];grow->copy->fast;fast->inc;
''')
fig('root-07','Pytania o dostęp, obecność i wariant wartości prowadzą do różnych mechanizmów języka.',r'''
q[label="Jakie pytanie zadaję?"];
b[label="Czy mogę chwilowo\nużyć cudzej wartości?\n&T"];
r[label="Czy zarządzany cel\nnadal jest obecny?\nRef<T>"];
o[label="Który wariant\nwartości posiadam?\nOption<T>"];
q->{b r o};
''')
fig('root-08','Modele opisują różne aspekty tego samego programu i wspólnie uzasadniają jego wykonanie.',r'''
p[label="Program Borka",fillcolor="#d5ebe6"];
a[label="AST\njaka struktura?"];
t[label="Typy\njakie operacje?"];
m[label="Model pamięci\njak długo żyją dane?"];
c[label="CFG\nktóra ścieżka?"];
ir[label="Kod natywny\nrealizacja wszystkich ustaleń"];
p->{a t m c};a->ir;t->ir;m->ir;c->ir;
''')

arrangements={
    'root-01':'{rank=same;a;b;c}',
    'front-01':'{rank=same;a;b;p} {rank=same;m;r}',
    'front-05':'',
    'front-08':'{rank=same;a;b;c} {rank=same;d;e}',
    'front-12':'{rank=same;a;b;c} {rank=same;d;e}',
    'front-13':'{rank=same;a;b;c} {rank=same;d;e}',
    'memory-11':'{rank=same;a;b;c} {rank=same;d;e}',
    'memory-12':'{rank=same;a;b} {rank=same;c;d}',
    'memory-13':'',
    'memory-14':'{rank=same;a;b;c} {rank=same;w;p;n}',
    'memory-15':'{rank=same;s;c;p} {rank=same;g1;g2}',
    'memory-16':'{rank=same;a;b;body} {rank=same;ra;rb;els;exit}',
    'back-15a':'{rank=same;a;b;c} {rank=same;d;e;err}',
    'back-17b':'{rank=same;a;b;c} {rank=same;cl1;cl2;els}',
    'back-18a':'{rank=same;a;b;o} {rank=same;r;l;sys} {rank=same;exe}',
    'back-19b':'{rank=same;s;dw;md} {rank=same;rt;dbg}',
    'back-20a':'{rank=same;open;text;check} {rank=same;analysis;out}',
    'back-20b':'{rank=same;a;b;c} {rank=same;d;e}',
    'root-04':'{rank=same;a;b}',
}
for key,ranks in arrangements.items():
    figures[key]=re.sub(r'rankdir=LR;', '',figures[key])+ '\n'+ranks
figures['front-14']+='\nw->x [style=invis]; w->y [style=invis];'
figures['back-19a']=figures['back-19a'].replace('root->{a b c d};',
    'root->{a b}; a->c[style=invis]; b->d[style=invis]; '
    'root->c[constraint=false]; root->d[constraint=false]; {rank=same;a;b} {rank=same;c;d}')
def wrap_label(m):
    label=m.group(1)
    if '|' in label or '{' in label:return m.group()
    lines=[]
    for line in label.split(r'\n'):
        lines.extend(textwrap.wrap(line,22,break_long_words=False,break_on_hyphens=False) or [''])
    return 'label="'+r'\n'.join(lines)+'"'
for key in figures:
    figures[key]=re.sub(r'label="([^"\n]*)"',wrap_label,figures[key])
(ROOT/'rysunki.json').write_text(json.dumps(figures,ensure_ascii=False,indent=2))
(ROOT/'podpisy.json').write_text(json.dumps(captions,ensure_ascii=False,indent=2))
print(f'{len(figures)} rysunków')
