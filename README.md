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
- Dos formas de ejecutar: un intérprete (`mou run`) y binarios nativos a través de C (`mou build`).
- Herramientas: `mou test` (con filtro por nombre, `--all` y `--native`), `mou fmt`, `mou repl`, `mou effects`, `mou fuzz` y `mou bind` (escribe las declaraciones de un header de C).
- El compilador escrito en Mou (`compiler/`): lexer, parser, cargador de módulos, verificador de tipos, bajada a MIR, ownership, efectos, constantes al compilar y emisor de C. Da lo mismo que el de Rust en cada etapa y **se compila a sí mismo**: el compilador que resulta vuelve a escribir exactamente el mismo C.

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

Hay dos caminos, y dan el mismo compilador.

Solo con un compilador de C (`cc`), a partir del compilador escrito en Mou:

```sh
./bootstrap.sh
./mouc c examples/tour.mou std src/runtime.c > tour.c && cc -O2 -o tour tour.c -lm && ./tour
```

Con Rust, que además trae el intérprete (`mou run`), la sesión interactiva, el formateador y los tests:

```sh
cargo build --release
./target/release/mou run examples/tour.mou
./target/release/mou build examples/tour.mou -o tour && ./tour
cargo test --release
```

## Nombre

Mou, o mou-lang para buscarlo. El comando es `mou` y los archivos terminan en `.mou`.

## Licencia

Mou se distribuye bajo dos licencias, a elección de quien lo use: [MIT](LICENSE-MIT) o [Apache 2.0](LICENSE-APACHE).
