# Przedmowa

Bork powstał z praktycznego zderzenia trzech przyzwyczajeń. W Kotlinie pisze się krótko. Blok kodu, funkcja przekazana na końcu wywołania i warunek zapisany jako wyrażenie nie wymagają od programisty osobnego rytuału. W Rustcie program natywny jest szybki i nie ma garbage collectora, ale czas życia każdej referencji trzeba opisać i obronić przed sprawdzaniem pożyczek. W C pamięć zwalnia się ręcznie, więc każdy `free` jest osobną decyzją, którą łatwo pomylić.

Bork wybiera inną zasadę, bo nawiasy klamrowe nie są tylko sposobem grupowania instrukcji, lecz oznaczają region pamięci. Napis albo tablica utworzona w takim bloku żyje tak długo, jak długo wykonanie pozostaje w tym bloku, a wyjście z bloku zwalnia cały bufor regionu naraz. Nie ma odśmiecania w tle ani ogólnych referencji, które można by schować w polu struktury, bo struktur definiowanych przez programistę w obecnej wersji języka nie ma.

Ta książka opisuje język taki, jaki przyjmuje kompilator w tym repozytorium, oraz sam kompilator, plik po pliku. To rozróżnienie jest ważne. Część kompilatora, która sprawdza program, rozumie wartości puste, operator `?:`, wykrzykniki `!!` oraz funkcję dopisaną na końcu wywołania. Generator kodu maszynowego tłumaczy węższy zestaw konstrukcji. Niektóre programy, które przechodzą sprawdzenie, dostają przy budowaniu zwykły komunikat błędu. Inne, i to trzeba powiedzieć wprost, kończą się awarią samego kompilatora. Dodatek C wymienia te miejsca.

Nazwa diagnostyki jest częścią projektu. Gdy program jest niepoprawny, komunikat mówi, w której fazie kompilator przerwał pracę. Język nie ma wyjątków, typu `Result` ani instrukcji `try`. Błąd programisty jest albo błędem kompilacji, albo przerwaniem programu w czasie działania, na przykład przy dzieleniu przez zero albo przy indeksie spoza tablicy.

Książka jest po polsku. Terminy, które w kodzie i w komunikatach kompilatora pozostają angielskie, podaję przy pierwszym użyciu i potem trzymam się jednego brzmienia. Słowo `move` w pliku źródłowym zostaje słowem `move`. W zdaniu objaśniającym mówię o przeniesieniu własności.

Jeśli chcesz najpierw napisać program, czytaj część pierwszą i trzymaj się przykładów, które dają się zbudować. Jeśli chcesz czytać kod kompilatora albo dopisać nową konstrukcję, część trzecia prowadzi przez repozytorium w kolejności, w której kolejne fazy naprawdę na sobie polegają.
