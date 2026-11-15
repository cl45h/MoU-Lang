# Mou

Español | [English](README.en.md)

Mou es un lenguaje de programación de sistemas en desarrollo. Busca combinar seguridad de memoria, efectos inferidos por el compilador y binarios nativos, con menos anotaciones que Rust y sin las trampas de C y C++.

Estado: experimental. El lenguaje cambia y todavía no sirve para producción.

```mou
import std.io

struct Point { x: int, y: int }

fn distance(a: Point, b: Point) -> int {
    let dx = a.x - b.x
    let dy = a.y - b.y
    dx * dx + dy * dy
}

fn main() ! io.Error {
    var total = 0
    for p in [Point(1, 2), Point(4, 6)] {
        total += distance(p, Point(0, 0))
    }
    io.print("total: {total}")?
}
```

## Qué tiene hoy

- Verificador de tipos con genéricos, traits y `dyn`.
- Ownership sin lifetimes: las referencias no se guardan, así que no hace falta anotarlas.
- Efectos inferidos (`alloc`, `panic`, `io`, `unsafe`, `block`) y anotaciones que los prohíben: `@noalloc`, `@nopanic`, `@noio`, `@nounsafe`, `@noblock`. Para `@nopanic` el compilador prueba rangos de índices y de aritmética.
- Constantes y expresiones `comptime` calculadas al compilar, y parámetros `comptime` de enteros y de tipos: `fn power(comptime n: int, x: int)` da una copia de la función por cada valor de `n`, y `fn empty(comptime T: type) -> [T]` se llama `empty(int)`.
- Llamadas a funciones de C con números, `bool`, strings y punteros opacos (`ptr`). `import c "math.h" as m` lee el header al compilar, sin declarar nada a mano.
- Tareas en hilos con concurrencia estructurada: `scope s { let a = s.spawn(f(x)) ... a.wait() }`. Una tarea lee las variables de afuera y no las puede modificar, así que no hay carreras de datos.
- Redes: `std.net` da TCP (escuchar, conectar, leer y escribir), UDP y búsqueda de nombres, escrito en Mou sobre los sockets de C, y `std.tls` da conexiones cifradas sobre OpenSSL, con el certificado del otro lado comprobado. Un servidor atiende cada conexión en una tarea. IPv4 e IPv6. Solo Linux y programas compilados.
- Librería estándar escrita en Mou: listas, mapas, tablas hash, texto con Unicode, matemática, archivos y directorios (`std.fs`), y `std.pool` con `Pool[T]` y `Handle[T]` para grafos y estructuras cíclicas.
- Python dentro del programa: `import py "math" as m` y después `m.sqrt(2.0)?.as_float()?`. Está hecho sobre `std.py`, una librería sobre la interfaz de C de CPython; las excepciones llegan como errores.
- Conversiones de enteros que no pierden nada en silencio: `as` conserva el valor o hace panic, y `u8.truncate(x)`, `u8.saturate(x)` y `u8.try_from(x)` dicen qué hacer cuando no entra.
- Dos formas de ejecutar: binarios nativos a través de C (`mou build`, y `mou run` que compila y ejecuta) y un intérprete escrito en Mou (`mou interp`), que no necesita compilador de C.
- Herramientas: `mou test` (con filtro por nombre y `--all`), `mou fmt`, `mou repl`, `mou effects`, `mou check`, `mou interp` y `mou bind` (escribe las declaraciones de un header de C).
- El compilador escrito en Mou (`compiler/`): lexer, parser, cargador de módulos, verificador de tipos, bajada a MIR, ownership, efectos, constantes al compilar y emisor de C. Es el único compilador y **se compila a sí mismo**: el compilador que resulta vuelve a escribir exactamente el mismo C.

## Qué le falta

El núcleo del lenguaje está completo y el compilador se compila a sí mismo. Lo que sigue está diseñado en [DESIGN.md](DESIGN.md) pero no hecho; cada punto es un trabajo grande por sí solo:

- Rendimiento parejo con C: hoy iguala en bucles, cálculo numérico, y structs (también unos dentro de otros), tuplas, opcionales y enums de números, `bool` y `char`, que ya van sin bloque de memoria. Sigue siendo varias veces más lento con strings y con structs y enums que guardan strings, arrays u opcionales, porque esos todavía van en bloques de memoria con etiqueta.
- Referencias guardadas en structs y regiones explícitas.
- TLS dentro del bucle de eventos: hoy varias conexiones a la vez se atienden con una tarea (un hilo) por conexión, o desde un solo hilo con `net.Server`, que entrega lo que pasa como eventos (`Open`, `Data`, `Done`) y no espera a un cliente lento al escribir; solo Linux. Tareas que se pueden cancelar, canales con `Chan[T]` y estado compartido con `Mutex[T]` ya se pueden.
- El resto del contexto: un `io` con implementación propia. `with files = tabla` (archivos de texto en memoria), `with alloc = arena`, `with io = lineas` (captura lo que se imprime), `with err = lineas` (lo que va a la salida de errores), `with input = lineas` (da lo que se lee), `with args = lista` y `with env = pares` ya están, y los tipos de función ya llevan efectos escritos (`fn(int) -> int @nopanic`).
- Tipos como valores de verdad y bloques `comptime` que generan código; las funciones que devuelven un tipo ya están en su forma simple (`fn Matrix(comptime T: type, comptime N: int) -> type { struct { ... } }`, usada como `Matrix(int, 6)`), igual que los parámetros `comptime` de enteros y de tipos (`empty(int)`), los arrays de largo fijo `[T; N]` y la reflexión con `meta.info(T)`.
- Un backend LLVM propio, SIMD y `soa`: hoy LLVM se usa a través de Clang (`CC=clang mou build`), que compila el mismo C. `import cpp`. En la otra dirección, C ya puede llamar funciones de Mou marcadas `@abi`.

El detalle, con los límites conocidos de cada parte, está en [DESIGN.md](DESIGN.md). La gramática está en [GRAMMAR.md](GRAMMAR.md).

## Compilar y probar

Hace falta solo un compilador de C (`cc`). El compilador está escrito en Mou y se construye a partir de una copia suya ya pasada a C (`seed/mou.c.gz`):

```sh
./bootstrap.sh
./mou run examples/tour.mou
./mou interp examples/tour.mou
./mou build examples/tour.mou -o tour && ./tour
./mou check examples/tour.mou
./mou test archivo.mou
./mou fmt archivo.mou
./mou repl
tests/suite.sh
```

`mou` busca la librería estándar y el runtime junto a su propio binario; `MOU_HOME` dice otro lugar. `run` compila el programa a un archivo temporal y lo ejecuta; `interp` lo corre en el intérprete, sin compilador de C (más lento, y sin canales, `Mutex` ni funciones de C). `tests/suite.sh` corre los tests del proyecto: compara lo que da cada programa con lo guardado en `tests/expected/`.

El primer compilador estaba escrito en Rust. Se borró cuando el de Mou pasó a hacer todo; sigue en la historia de git, en la etiqueta `ultimo-con-rust`.

## Nombre

Mou, o mou-lang para buscarlo. El comando es `mou` y los archivos terminan en `.mou`.

## Licencia

Mou se distribuye bajo dos licencias, a elección de quien lo use: [MIT](LICENSE-MIT) o [Apache 2.0](LICENSE-APACHE).



Mou is a systems programming language under development. It aims to combine memory safety, effects inferred by the compiler and native binaries, with fewer annotations than Rust and without the traps of C and C++.

Status: experimental. The language changes and is not ready for production.

```mou
import std.io

struct Point { x: int, y: int }

fn distance(a: Point, b: Point) -> int {
    let dx = a.x - b.x
    let dy = a.y - b.y
    dx * dx + dy * dy
}

fn main() ! io.Error {
    var total = 0
    for p in [Point(1, 2), Point(4, 6)] {
        total += distance(p, Point(0, 0))
    }
    io.print("total: {total}")?
}
```

## What it has today

- A type checker with generics, traits and `dyn`.
- Ownership without lifetimes: references are never stored, so they need no annotations.
- Inferred effects (`alloc`, `panic`, `io`, `unsafe`, `block`) and annotations that forbid them: `@noalloc`, `@nopanic`, `@noio`, `@nounsafe`, `@noblock`. For `@nopanic` the compiler proves index and arithmetic ranges.
- Constants and `comptime` expressions computed at compile time, and `comptime` parameters for integers and types: `fn power(comptime n: int, x: int)` gives one copy of the function per value of `n`, and `fn empty(comptime T: type) -> [T]` is called as `empty(int)`.
- Calls to C functions with numbers, `bool`, strings and opaque pointers (`ptr`). `import c "math.h" as m` reads the header at compile time, with nothing declared by hand.
- Tasks on threads with structured concurrency: `scope s { let a = s.spawn(f(x)) ... a.wait() }`. A task reads the variables outside it and cannot modify them, so there are no data races.
- Networking: `std.net` gives TCP (listen, connect, read and write), UDP and name lookup, written in Mou on top of C sockets, and `std.tls` gives encrypted connections on top of OpenSSL, with the certificate of the other side checked. A server handles each connection in a task. IPv4 and IPv6. Linux and compiled programs only.
- A standard library written in Mou: lists, maps, hash tables, text with Unicode, math, files and directories (`std.fs`), and `std.pool` with `Pool[T]` and `Handle[T]` for graphs and cyclic structures.
- Python inside the program: `import py "math" as m` and then `m.sqrt(2.0)?.as_float()?`. It is built on `std.py`, a library on top of the C interface of CPython; exceptions arrive as errors.
- Integer conversions that never lose anything silently: `as` keeps the value or panics, and `u8.truncate(x)`, `u8.saturate(x)` and `u8.try_from(x)` say what to do when it does not fit.
- Two ways to run: native binaries through C (`mou build`, and `mou run` which compiles and runs) and an interpreter written in Mou (`mou interp`), which needs no C compiler.
- Tools: `mou test` (with a name filter and `--all`), `mou fmt`, `mou repl`, `mou effects`, `mou check`, `mou interp` and `mou bind` (writes the declarations of a C header).
- The compiler written in Mou (`compiler/`): lexer, parser, module loader, type checker, lowering to MIR, ownership, effects, compile-time constants and C emitter. It is the only compiler and **it compiles itself**: the resulting compiler writes exactly the same C again.

## What is missing

The core of the language is complete and the compiler compiles itself. What follows is designed in [DESIGN.md](DESIGN.md) but not built; each item is a large job on its own:

- Performance on par with C: today it matches C in loops, numeric code, and structs (nested ones too), tuples, optionals and enums of numbers, `bool` and `char`, which already live without a memory block. It is still several times slower with strings and with structs and enums that hold strings, arrays or optionals, because those still go in tagged memory blocks.
- References stored in structs, and explicit regions.
- TLS inside the event loop: today several connections at once are handled with one task (one thread) per connection, or from a single thread with `net.Server`, which hands over what happens as events (`Open`, `Data`, `Done`) and does not wait for a slow client when writing; Linux only. Cancellable tasks, channels with `Chan[T]` and shared state with `Mutex[T]` already work.
- The rest of the context: an `io` with an implementation of your own. `with files = table` (text files in memory), `with alloc = arena`, `with io = lines` (captures what is printed), `with err = lines` (what goes to the error output), `with input = lines` (provides what is read), `with args = list` and `with env = pairs` are already there, and function types already carry written effects (`fn(int) -> int @nopanic`).
- Types as real values and `comptime` blocks that generate code; functions that return a type are already there in their simple form (`fn Matrix(comptime T: type, comptime N: int) -> type { struct { ... } }`, used as `Matrix(int, 6)`), as are `comptime` parameters for integers and types (`empty(int)`), fixed-length arrays `[T; N]` and reflection with `meta.info(T)`.
- An LLVM backend of its own, SIMD and `soa`: today LLVM is used through Clang (`CC=clang mou build`), which compiles the same C. `import cpp`. In the other direction, C can already call Mou functions marked `@abi`.

The details, with the known limits of each part, are in [DESIGN.md](DESIGN.md). The grammar is in [GRAMMAR.md](GRAMMAR.md). Both documents are in Spanish.

## Build and test

All it takes is a C compiler (`cc`). The compiler is written in Mou and is built from a copy of itself already turned into C (`seed/mou.c.gz`):

```sh
./bootstrap.sh
./mou run examples/tour.mou
./mou interp examples/tour.mou
./mou build examples/tour.mou -o tour && ./tour
./mou check examples/tour.mou
./mou test file.mou
./mou fmt file.mou
./mou repl
tests/suite.sh
```

`mou` looks for the standard library and the runtime next to its own binary; `MOU_HOME` names another place. `run` compiles the program to a temporary file and runs it; `interp` runs it in the interpreter, without a C compiler (slower, and without channels, `Mutex` or C functions). `tests/suite.sh` runs the tests of the project: it compares what each program gives with what is kept in `tests/expected/`.

The first compiler was written in Rust. It was removed once the one in Mou did everything; it is still in the git history, under the tag `ultimo-con-rust`.

## Name

Mou, or mou-lang when searching for it. The command is `mou` and files end in `.mou`.

## License

Mou is distributed under two licenses, at the user's choice: [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE).
