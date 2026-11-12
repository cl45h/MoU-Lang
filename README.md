# Mou

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
- Constantes y expresiones `comptime` calculadas al compilar.
- Llamadas a funciones de C con números, `bool`, strings y punteros opacos (`ptr`).
- Tareas en hilos con concurrencia estructurada: `scope s { let a = s.spawn(f(x)) ... a.wait() }`. Una tarea lee las variables de afuera y no las puede modificar, así que no hay carreras de datos.
- Dos formas de ejecutar: un intérprete (`mou run`) y binarios nativos a través de C (`mou build`).
- Herramientas: `mou test`, `mou fmt`, `mou repl`, `mou effects` y `mou fuzz`.
- El compilador escrito en Mou (`compiler/`): lexer, parser, cargador de módulos, verificador de tipos, bajada a MIR, ownership, efectos, constantes al compilar y emisor de C. Da lo mismo que el de Rust en cada etapa y **se compila a sí mismo**: el compilador que resulta vuelve a escribir exactamente el mismo C.

## Qué le falta

El núcleo del lenguaje está completo y el compilador se compila a sí mismo. Lo que sigue está diseñado en [DESIGN.md](DESIGN.md) pero no hecho; cada punto es un trabajo grande por sí solo:

- Rendimiento parejo con C: hoy iguala en bucles, cálculo numérico y structs de campos simples (números y `bool`) y opcionales de números, que ya van sin bloque de memoria. Sigue siendo varias veces más lento con enums, strings y structs que guardan otros valores, porque esos todavía van en bloques de memoria con etiqueta.
- Referencias guardadas en structs y regiones explícitas.
- Entrada y salida sin bloquear (tareas que se pueden cancelar, canales con `Chan[T]` y estado compartido con `Mutex[T]` ya se pueden).
- El contexto (`with alloc = arena`) y los efectos en los tipos de función.
- Parámetros `comptime`, tipos como valores y reflexión.
- Structs con layout de C, buffers y callbacks en las llamadas a C (números, `bool`, strings y punteros opacos ya se pueden), y leer los headers en vez de declarar las funciones a mano.
- Backend LLVM, SIMD y `soa`; `import cpp` e `import py`.
- Los bloques `with`, que son parte del contexto: se leen, pero el compilador avisa que no los soporta.

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
