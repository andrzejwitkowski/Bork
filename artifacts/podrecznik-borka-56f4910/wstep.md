# 1. Jak czytać kompilator Borka

## Książka przypięta do kodu

Ten podręcznik wyjaśnia budowę kompilatora Borka na podstawie gałęzi `main`, pobranej 2 listopada 2026 roku. Punktem odniesienia jest commit `56f4910a24b689087fe2dee30247651b7b432268`, zapisany 30 października 2026 roku. Odsyłacze do źródeł prowadzą do tej konkretnej wersji. Dzięki temu opis można zestawić z kodem także wtedy, gdy projekt pójdzie dalej.

Zakładamy znajomość programowania: funkcji, struktur danych, stosu wywołań oraz podstaw wskaźników. Znajomość Rusta pomaga przy czytaniu implementacji, ale nie jest warunkiem rozumienia koncepcji. Terminy takie jak unifikacja, SSA czy monomorfizacja są wprowadzane przy pierwszym użyciu. Po przeczytaniu książki powinieneś umieć prześledzić program przez kompilator, uzasadnić jego diagnozę i wskazać miejsca, które trzeba zmienić, dodając nową cechę.

Ważne rozróżnienie dotyczy trzech rodzajów materiału. Programy podpisane ścieżką `programs/...` pochodzą z korpusu projektu. Fragmenty pokazują wybraną część programu i mogą wymagać otaczających deklaracji. Schematy HIR, LLVM i algorytmów służą wyjaśnieniu; są oznaczone jako schemat lub pseudokod, jeżeli nie stanowią dosłownego wydruku narzędzia. Nie należy kopiować schematu LLVM jako gotowego modułu.

## Dwa sposoby korzystania

Przy pierwszej lekturze czytaj kolejno rozdziały 1–8, żeby poznać reprezentacje programu i język. Rozdziały 9–14 wyjaśniają model pamięci, który najmocniej wyróżnia Borka. Rozdziały 15–20 prowadzą przez backend i narzędzia. Rozdział 21 składa te wiadomości w jeden przebieg. Ćwiczenia z rozdziału 22 pomagają utrwalić intuicję.

Przy pracy nad konkretnym problemem zacznij od indeksu tematycznego albo rozdziału 23. Jeżeli zastanawiasz się, dlaczego `Ref<Node>` wymaga guarda, przejdź do rozdziału 12. Jeżeli typ jest poprawny, ale nie powstaje binarka, sprawdź rozdziały 15 i 18. Jeżeli po dodaniu nowego wariantu HIR rozjeżdża się plan regionów, potrzebne będą rozdziały 10, 17 i 20. Spis treści, zakładki czytnika PDF oraz indeks mają wspierać te różne sposoby czytania.

[[diagram:root-01|Mapa lektury|Najpierw poznaj język i reprezentacje, następnie model pamięci, a na końcu backend. Studium przypadku łączy wszystkie trzy ścieżki.]]

## Kompilator jako ciąg zobowiązań

Kompilator przekształca program w kolejne reprezentacje i przy okazji uzasadnia coraz więcej jego własności. Parser odpowiada, czy tekst ma dozwoloną strukturę. Rozwiązywanie nazw ustala, do której deklaracji należy użycie identyfikatora. Kontrola typów sprawdza, jakie operacje mają sens. Analiza własności ustala, czy używana wartość nadal jest dostępna i czy pożyczka nie przeżywa właściciela. Plan pamięci wybiera sposób przechowywania obiektów. Backend produkuje instrukcje, które realizują te ustalenia.

Weźmy wyrażenie `p.x + 1`. Dla parsera jest to dodawanie odczytu pola i literału. Kontrola typów musi wiedzieć, czym jest `p`, czy ma pole `x` i jaki typ przyjąć dla `1`. Analiza własności sprawdza, czy `p` nie zostało wcześniej przeniesione. Backend potrzebuje indeksu pola w strukturze oraz instrukcji dodawania zgodnej z jego typem. Ten sam krótki zapis uruchamia więc kilka różnych rodzajów rozumowania.

Podział na fazy ogranicza liczbę pytań rozwiązywanych jednocześnie. Generator kodu nie powinien ponownie zgadywać, którą funkcję autor miał na myśli. Gdy dostaje typowany HIR, część informacji jest już utrwalona. Kompilator może jednak zachować dodatkowe reprezentacje: w Borku analiza własności nadal odwiedza AST, otrzymując typy i rozstrzygnięcia członków z kontroli typów. Dlatego rzeczywisty przepływ danych ma rozgałęzienia; nie jest jedynie prostą kolejką drzew.

[[diagram:root-02|Przepływ danych w aktualnym frontendzie|Typeck wytwarza HIR oraz dane dla analizy własności AST. Dalsze planowanie działa dopiero po sprawdzeniu diagnostyk.]]

## Język, implementacja i granica backendu

Bork ma ekspresyjną składnię zbliżoną do Kotlina, ale własny model pamięci. Bloki uczestniczą w organizowaniu regionów. Wartości posiadające zasoby podlegają regułom własności; dane mogą wymagać skopiowania do innego regionu albo wybrania docelowej areny już w chwili konstrukcji. Runtime obsługuje areny i zarządzane odwołania, bez śledzącego garbage collectora.

Z tego opisu nie wynika, że każdy blok zawsze wywołuje alokator ani że każde zwolnienie jest pojedynczą instrukcją o stałym koszcie. Kompilator może pominąć niepotrzebną arenę. Zwolnienie stron i obsługa zapisanych odwołań mają własny koszt. Rysunki regionów przedstawiają zależności czasu życia, a dopiero rozdziały o implementacji pokazują, kiedy stają się one konkretnymi strukturami runtime'u.

W projekcie istnieje też różnica między akceptacją frontendu a obsługą natywną. `bork plik.bork` sprawdza program. `bork build plik.bork` dodatkowo wymaga obsługi przez backend, funkcji wejściowej i działającego toolchainu. Przykładem świadomego ograniczenia są trailing closures: ich składnia jest rozpoznawana, ale kod natywny tej wersji ich nie obsługuje. Traity i wywołania statyczne są natomiast już zaimplementowane, choć niektóre starsze opisy projektu mówią inaczej.

## Jak oceniać decyzję projektową

Każdą decyzję warto rozpatrywać na trzech poziomach. Pierwszy to semantyka: co ma znaczyć program? Drugi to reprezentacja: jak zapisać potrzebną informację w AST, HIR i strukturach analizy? Trzeci to realizacja: jakie instrukcje oraz wywołania runtime'u mają powstać? Na przykład brak wartości może być pojęciem językowym, wariantem enumu i konkretnym tagiem w pamięci. Te poziomy są powiązane, ale nie muszą mieć identycznej postaci.

W książce szczególnie pilnujemy różnicy między uzasadnieniem ogólnym a własnością aktualnego Borka. Rozdział o unifikacji wyjaśnia uniwersalny algorytm, a potem wskazuje ograniczenia konkretnego solvera. Rozdział o referencjach wyjaśnia silne i słabe krawędzie, ale nie przypisuje Borkowi dowolnego systemu `shared_ptr`. Zrozumienie implementacji wymaga obu perspektyw: pojęcia tłumaczą cel, kod ujawnia rzeczywiste granice.

Ćwiczenie: błąd „unknown field” pojawił się przed emisją LLVM. Która informacja była niezbędna do jego wykrycia? Odpowiedź: typ odbiorcy i lista jego pól. Sama poprawność składni `p.x` nie wystarcza; naturalnym miejscem diagnozy jest kontrola typów i rozwiązywanie dostępu do członka.

Źródła: [frontend::check](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/frontend.rs), [CheckResult i reprezentacje](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/hir/mod.rs), [bramka backendu](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/codegen_gate.rs), [CLI](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/main.rs).
