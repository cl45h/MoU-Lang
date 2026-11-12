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
- Efectos inferidos (`alloc`, `panic`, `io`, `unsafe`) y anotaciones que los prohíben: `@noalloc`, `@nopanic`, `@noio`. Para `@nopanic` el compilador prueba rangos de índices y de aritmética.
- Constantes y expresiones `comptime` calculadas al compilar.
- Llamadas a funciones de C con números y `bool`.
- Dos formas de ejecutar: un intérprete (`mou run`) y binarios nativos a través de C (`mou build`).
- Herramientas: `mou test`, `mou fmt`, `mou repl`, `mou effects` y `mou fuzz`.
- La primera mitad del compilador escrito en Mou (`compiler/`): el lexer, el parser, el cargador de módulos, el verificador de tipos y la bajada a MIR, que dan lo mismo que los de Rust y ya se procesan a sí mismos.

## Qué le falta

- Rendimiento parejo con C: hoy iguala en bucles y cálculo numérico, pero es varias veces más lento en recursión y en arrays de structs.
- Referencias guardadas en structs, concurrencia, y punteros y structs en las llamadas a C.

El detalle, con los límites conocidos de cada parte, está en [DESIGN.md](DESIGN.md). La gramática está en [GRAMMAR.md](GRAMMAR.md).

## Compilar y probar

Hace falta Rust y un compilador de C (`cc`).

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
