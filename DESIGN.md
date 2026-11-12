# Mou: diseño del lenguaje (v0.1, borrador)

Nombre: **Mou**, por las primeras letras del apellido de su autor. Extensión `.mou`, herramienta `mou`. Para buscarlo o publicarlo, `mou-lang`: "Mou" a secas choca con un editor de Markdown y con la sigla MoU.

Objetivo: un solo lenguaje desde scripts hasta kernels. Seguro como Rust, tan rápido como C o más, interoperable con C, C++ y Python, y portable a cualquier plataforma.

---

## 1. Principios (en orden: si dos chocan, gana el de arriba)

1. **Seguro por defecto.** Sin comportamiento indefinido fuera de `unsafe`.
2. **Sin costos ocultos.** Toda alocación, copia cara o bloqueo es visible o está permitido por el perfil del módulo.
3. **Gradual.** El mismo código puede empezar sin anotaciones y endurecerse función por función.
4. **Núcleo chico.** Lo que puede ser librería no entra al lenguaje.
5. **Una sola forma obvia** de hacer cada cosa.

---

## 2. Sintaxis base

Orientada a expresiones, llaves, sin punto y coma, tipo después del nombre.

```mou
import std.io

struct Point { x: f64, y: f64 }

enum Shape {
    Circle(center: Point, radius: f64),
    Rect(min: Point, max: Point),
}

fn area(f: Shape) -> f64 {
    match f {
        Circle(_, r) => 3.14159 * r * r,
        Rect(a, b)    => (b.x - a.x) * (b.y - a.y),
    }
}

fn main() ! io.Error {
    let shapes = [Shape.Circle(Point(0, 0), 1.0)]
    for f in shapes {
        io.print("area = {area(f)}")?
    }
}
```

- `let` inmutable, `var` mutable.
- `if`, `match`, y bloques son expresiones. La última expresión de un bloque es su valor.
- Interpolación de strings con `{}`.
- `defer` para limpieza al salir del scope.
- Palabras clave, librería estándar y ejemplos en inglés.

---

## 3. Tipos

| Categoría | Tipos |
|---|---|
| Enteros | `i8 i16 i32 i64 i128`, `u8 ... u128`, `isize usize`; `int` = `i64` |
| Flotantes | `f32 f64`; `float` = `f64` |
| Otros primitivos | `bool`, `char` (escalar Unicode), `str` (UTF-8) |
| Compuestos | `[T; N]` array, `[T]` slice (vista), `(A, B)` tupla |
| Usuario | `struct`, `enum` (tipos suma con datos), `trait` |
| Ausencia | `T?` (opcional). No existe `null` |
| Error | `T ! E` (resultado o error) |
| Vectores SIMD | `vec[N, T]`, primitivos del lenguaje |

Reglas:
- Un literal entero toma el tipo que pide el contexto (`let x: f64 = 0` es válido) y se verifica que entre en ese tipo. Sin contexto es `int`; un literal flotante es `f64`.
- Sin conversiones implícitas entre tipos numéricos. `x as i32` solo cuando no pierde información; lo demás con `x.truncate()`, `x.saturate()`, `x.try_into[i32]()`.
- Toda variable se inicializa antes de leerse (verificado en compilación).
- Aritmética entera: `+ - *` atrapan overflow (panic). Operadores explícitos: `+%` envolvente, `+|` saturante.
- Indexado `xs[i]` verifica límites salvo que el compilador pruebe que es válido. `xs.get(i)` devuelve `T?`.
- Layout: `struct` puede reordenar campos; `@c struct` usa layout de C; `@packed struct` sin padding.
- `soa [T]`: mismo tipo lógico que `[T]` pero almacenado como un array por campo.

### Genéricos y traits

```mou
trait Ord { fn cmp(self, other: Self) -> Order }

fn max[T: Ord](a: T, b: T) -> T {
    if a.cmp(b) == Order.Greater { a } else { b }
}
```

Los genéricos se verifican en la definición (no al instanciar). Se monomorfizan por defecto; `dyn Trait` para despacho dinámico explícito.

---

## 4. Modelo de memoria

Semántica de valores con ownership. Las referencias son de **segunda clase** por defecto: existen como parámetros y variables locales, pero no se guardan en structs ni escapan. Eso elimina las anotaciones de lifetime en casi todo el código.

### Convenciones de paso de parámetros

| Forma | Significado | En la llamada |
|---|---|---|
| `p: T` | préstamo inmutable (por defecto) | `f(x)` |
| `p: mut T` | préstamo mutable exclusivo | `f(mut x)` |
| `p: own T` | transfiere ownership | `f(own x)` o `f(x)` si es el último uso |

```mou
fn scale(p: mut Point, k: f64) { p.x *= k; p.y *= k }

var a = Point(1, 2)
scale(mut a, 3.0)      // the mutation is visible at the call site
```

**Ley de exclusividad:** mientras existe un acceso `mut` a un valor, no existe ningún otro acceso a ese valor. Los préstamos son por campo: tomar `mut a.x` no bloquea `a.y`.

### Devolver referencias

Una función puede devolver una parte de `self` (o de su primer parámetro, si no es un método) sin anotaciones. Con `ref T` el resultado se lee; con `ref mut T` además se puede escribir, y la escritura llega al valor original:

```mou
fn first(self) -> ref T { self.data[0] }
fn at(mut self, i: int) -> ref mut T { self.data[i] }

v.at(2) = x            // writes into `v`
v.at(0).name = "a"
bump(mut v.at(1))
```

El resultado es la última expresión del cuerpo y tiene que ser el parámetro, un campo o un elemento suyo, u otra llamada `ref` sobre él. Mientras se usa, cuenta como un acceso al valor del que salió.

Cualquier caso más complejo (referencias guardadas en structs, varias fuentes posibles) requiere regiones explícitas y solo está disponible en el perfil `system`:

```mou
struct View[region r] { data: ref[r] [u8] }
```

### Heap

- `Box[T]`: dueño único. `Rc[T]` / `Arc[T]`: compartido con conteo.
- `Arena`: alocación por bloques, se libera entera.
- `Handle[T]` + `Pool[T]`: índices generacionales para grafos, listas dobles y estructuras cíclicas, verificados en runtime con un check barato.
- Construcción in-place garantizada: los valores de retorno se escriben directo en su destino. Un move nunca es observable como copia.

### Sin unwinding

`panic` es para bugs y aborta (o llama a un handler configurable en freestanding). Los errores recuperables son valores.

---

## 5. Errores

```mou
fn load_config(path: Path) -> Config ! ConfigError {
    let text = fs.read(path)?            // propagates, converting the error with From
    let cfg = parse(text) catch e {    // handled locally
        return ConfigError.Syntax(e)
    }
    cfg
}
```

`T ! E` es un tipo suma común. `?` propaga. Dentro de una función `-> T ! E`, devolver un valor de tipo `E` es devolver el error. Ignorar un resultado con error es error de compilación.

---

## 6. Efectos y contexto

Cada función tiene un conjunto de efectos, **inferido** por el compilador. Efectos base:

| Efecto | Significa |
|---|---|
| `alloc` | puede alocar en heap |
| `panic` | puede abortar (overflow, índice, `unwrap`) |
| `io` | interactúa con el exterior |
| `block` | puede bloquear o suspender |
| `unsafe` | contiene operaciones no verificadas |

Se restringen con anotaciones, y el compilador verifica toda la cadena de llamadas:

```mou
@noalloc @nopanic
fn average(xs: [f64]) -> f64
    where xs.len > 0
{
    var s = 0.0
    for x in xs { s += x }
    s / xs.len as f64
}
```

En una función `@nopanic`, `xs[i]` solo compila si el compilador prueba el rango (por `where`, por el `for`, o por un `if` previo), y `+` entero exige `+%`, `+|` o prueba de rango.

`where` declara precondiciones: se prueban en el sitio de llamada si es posible; si no, se verifican en runtime (y entonces la llamada tiene efecto `panic`).

### Contexto

Las capacidades (`alloc`, `io`) llegan por un contexto implícito que el llamador puede reemplazar:

```mou
with alloc = arena {
    let list = List[int].new()     // uses the arena
}
```

- No hay autoridad ambiente: una librería no puede abrir un socket si no recibe `io`.
- Esto reemplaza a `async`: el mismo código corre bloqueante, con event loop o con hilos según la implementación de `io` inyectada. No hay funciones coloreadas.
- El contexto se resuelve estáticamente cuando es posible (cero costo).

---

## 7. Perfiles: los tres niveles

Se declaran por módulo (o por paquete en el manifiesto). No cambian la semántica, solo qué puede quedar implícito.

| | `script` | `app` (por defecto) | `system` |
|---|---|---|---|
| Tipos en firmas | inferidos | obligatorios en `pub` | obligatorios siempre |
| Copias implícitas de tipos con heap | sí (CoW / Rc optimizado) | no, `.copy()` | no |
| Alocador ambiente | sí | sí | no, contexto explícito |
| Regiones explícitas | no | no | sí |
| `unsafe` | no | no | sí |
| Runtime / libc requeridos | sí | sí | no (freestanding) |

Un módulo `script` puede llamar a todo. Un módulo `system` solo puede llamar a funciones cuyos efectos permite. Optimizar es mover una función a un módulo más estricto, no reescribirla en otro lenguaje.

---

## 8. Ejecución en compilación

```mou
fn Matrix(comptime T: type, comptime N: usize) -> type {
    struct { data: [T; N * N] }
}

comptime {
    for field in info(Config).fields {
        generate_parser(field)
    }
}
```

- Los tipos son valores en `comptime`.
- Reflexión con `info(T)`.
- Reemplaza macros, templates, `constexpr` y generadores de código.
- El código `comptime` no tiene `io` salvo permiso explícito del build (builds reproducibles).

---

## 9. Concurrencia

Estructurada: las tareas no sobreviven a su scope.

```mou
scope s {
    let a = s.spawn(download(url1))
    let b = s.spawn(download(url2))
    process(a.wait()?, b.wait()?)
}
```

- El ownership impide data races: lo que cruza a otra tarea se mueve (`own`) o es inmutable o es `Arc`/`Mutex`.
- Canales en la librería estándar.

---

## 10. Módulos, interop y herramientas

- Un archivo es un módulo. `pub` exporta. Sin headers, sin preprocesador.
- Interop:
  ```mou
  import c   "sqlite3.h"       // funciones unsafe, tipos con layout C
  import cpp "opencv2/core.hpp"
  import py  "numpy" as np     // solo perfiles script/app
  ```
- ABI estable opt-in con `pub @abi`.
- Una sola herramienta: `mou run | build | test | fmt | pkg | repl` (hoy existen `run`, `build`, `test`, `fmt`, `repl`, `effects` y `fuzz`; `build` acepta `-l nombre` para enlazar una librería de C).
- Cross-compilación: `mou build --target=aarch64-linux`.

---

## 11. Arquitectura del compilador

```
fuente -> lexer/parser -> AST
       -> resolución de nombres + inferencia de tipos y efectos -> HIR tipado
       -> evaluación comptime + monomorfización
       -> MIR (CFG) -> verificador de ownership/exclusividad, prueba de rangos
       -> backends:
            intérprete de MIR   (repl, scripts, comptime)
            emisor de C         (portabilidad total, primer backend nativo)
            backend debug propio (compilación rápida)
            LLVM                (release)
            WASM
```

---

## 12. Hoja de ruta

| Hito | Entregable |
|---|---|
| M0 | Esta especificación + gramática formal |
| M1 | Lexer y parser (hechos, en `src/`), verificador de tipos (hecho, en `src/check.rs`): núcleo, funciones, structs, enums, `impl` y alias genéricos, traits con supertraits y métodos por defecto. Módulos (hecho, en `src/load.rs`) y primera librería estándar escrita en el lenguaje (hecha, en `std/`). `dyn Trait`, traits genéricos, `dyn` de traits genéricos, varios `impl` de un trait genérico para un tipo y métodos genéricos dentro de un trait (hechos) |
| M2 | Intérprete de MIR: corren programas reales con `mou run` (hecho, en `src/mir.rs` y `src/interp.rs`); `repl` (hecho, en `src/repl.rs`) |
| M3 | Verificador de ownership y exclusividad (hecho: exclusividad en `src/check.rs`, inicialización y movimientos en `src/own.rs`). `ref` y `ref mut` de retorno (hechos). Pendiente: regiones explícitas y referencias guardadas en structs |
| M4 | Emisor de C: binarios nativos con `mou build` (hecho, en `src/cgen.rs` y `src/runtime.c`). Los enteros de hasta 64 bits, los booleanos y los `f64` ya son variables de máquina, y los arrays de escalares y de structs de escalares guardan sus datos sin etiqueta. Pendiente: structs sueltos, enums y strings con layout propio, y structs con campos que no son escalares |
| M5 | Efectos y perfiles (hechos en `src/effects.rs`: `alloc`, `panic` e `io` inferidos, `@noalloc`, `@nopanic`, `@noio`, prueba de rangos, y `system` sin alocador). Pendiente: el contexto (`with alloc = arena`), los efectos `block` y `unsafe`, y el resto de las reglas de los perfiles |
| M6 | `comptime` y reflexión. Hecho (en `src/comptime.rs`): las constantes globales y las expresiones `comptime` se calculan al compilar. Pendiente: parámetros `comptime`, tipos como valores e `info(T)` |
| M7 | `import c`. Hecho: llamar a funciones de C declaradas a mano, con números y `bool`. Pendiente: leer el header (vía libclang), strings, punteros, structs con layout de C y callbacks |
| M8 | Backend LLVM, SIMD, `soa` |
| M9 | Autoalojamiento del compilador: hecho. El compilador entero está escrito en Mou (`compiler/`), da el mismo C que el de Rust y se compila a sí mismo; `./bootstrap.sh` lo construye solo con un compilador de C, a partir de `seed/mouc.c.gz`. Pendiente: `import cpp`, `import py` |

Hay dos implementaciones del compilador que dan lo mismo: la primera, en Rust (`src/`), y la escrita en Mou (`compiler/`), que se compila a sí misma. Los tests exigen que coincidan etapa por etapa, así que cada cambio del lenguaje se hace en las dos.

---

## 13. Decisiones abiertas

1. Alcance exacto del probador de rangos: qué se prueba en compilación y qué cae a check de runtime. Hay que fijarlo en la especificación para que sea predecible.
2. Estrategia de memoria automática del perfil `script`: conteo de referencias con elisión (propuesto) contra inferencia de regiones pura.
3. Modelo formal de aliasing para `unsafe`.
4. `as` sin pérdida: la sección 3 dice que `as` solo vale cuando no pierde información, pero los ejemplos usan `xs.len as f64` y `len as int`. El verificador acepta hoy cualquier conversión entre numéricos; hay que decidir la regla exacta y aplicarla.
5. Indexar un `str`: el verificador solo acepta rangos (`s[..i]`), no un entero, porque `str` es UTF-8. Falta confirmarlo como regla del lenguaje.

6. `as` fuera de rango: el intérprete hace panic cuando el valor no entra en el tipo destino (`300 as u8`, o un flotante enorme a entero). Es la lectura más segura de la decisión 4, pero sigue abierta.
7. `io.print` escribe una línea (agrega el salto). Falta decidir si habrá una variante sin salto.
8. Un `int` devuelto por `main` es el código de salida del proceso; un error devuelto por `main` se informa y sale con 1; un panic sale con 101.
9. Literales en operaciones: `x + 300` con `x: u8` compila y hace panic al ejecutar. Debería rechazarse en compilación, igual que `let x: u8 = 300`.
10. `[T]` como buffer que crece: la sección 3 dice que `[T]` es una vista, pero hoy un `[T]` es dueño de sus datos y tiene `push` y `pop`, porque hacía falta una primitiva para escribir `List[T]` en el propio lenguaje. Cuando existan referencias y regiones, la primitiva debería ser un buffer aparte y `[T]` volver a ser solo una vista. El código que usa `std.list` no cambia.
11. Un `impl` para un tipo primitivo vale para todo el programa: si dos módulos escriben `impl Ord for int`, el segundo es un error. Falta decidir la regla de coherencia (quién puede implementar qué trait para qué tipo).
12. Privacidad: `pub` se exige a funciones, tipos, constantes, traits y métodos usados desde otro módulo. Los campos de un struct son visibles desde cualquier módulo; falta decidir si llevan `pub`.
13. Un tipo genérico tiene un solo `impl` por método para todos sus argumentos: `impl List[int]` (especialización) se rechaza.

### Qué ejecuta hoy el intérprete

`mou run archivo.mou` verifica tipos, traduce a MIR (un grafo de control por función, visible con `mou mir`) y ejecuta `main`.

Ejecuta: todo lo que el verificador de tipos cubre. Aritmética entera con detección de overflow, `+%` y `+|`, división por cero, límites de índices y de rangos, conversiones `as`, structs, enums, tuplas, arrays y slices, opcionales, errores con `?` y `catch`, `match` con todos los patrones, `for`/`while`/`loop`, `defer`, condiciones `where` (verificadas al entrar a la función), valores por defecto de parámetros y campos, constantes globales, funciones como valores, métodos, structs, enums e `impl` genéricos, módulos propios y de la librería estándar, `panic(mensaje)`, `assert`, `push`, `pop` y `get` de `[T]`, y las funciones de los módulos `std.io` y `std.raw`. Un panic informa archivo (el del módulo donde ocurre), línea, columna y la pila de llamadas.

Los parámetros `mut` se pasan por copia de entrada y de salida. La ley de exclusividad, que ya se verifica, hace que eso sea indistinguible de pasar una referencia.

No ejecuta todavía (lo rechaza antes de correr, con el lugar exacto): closures sin tipos en los parámetros, bloques `scope` y `with`, declaraciones anidadas, las llamadas a funciones de C (solo en el binario), los demás métodos de tipos primitivos y `mut` sobre un rango (`f(mut xs[1..])`).

Límites conocidos: los enteros se guardan en 128 bits con signo, así que un `u128` de 2^127 o más produce un panic que lo dice en vez de un resultado; un slice es una copia, no una vista; las closures capturan por valor y no pueden modificar lo capturado.

### Genéricos y traits: cómo están hechos

Una función genérica se verifica dos veces. Primero su definición, una sola vez, contra las restricciones: dentro de `fn max[T: Ord]` solo se puede hacer con un `T` lo que `Ord` declara, así que los errores aparecen en la definición y no en cada uso. Después se verifica y se traduce una copia por cada combinación de tipos con la que se la llama (monomorfización): `max(3, 9)` y `max(Version(1, 4), Version(1, 10))` son dos funciones distintas en el MIR, y la llamada a `cmp` dentro de cada una ya está resuelta al método concreto. No hay despacho dinámico ni costo en ejecución.

Los tipos se infieren de los argumentos o se escriben: `max[int](7, 2)`.

Los structs y enums genéricos (`struct Box[T]`, `enum Tree[T]`) llevan sus argumentos en el tipo: `Box[int]` y `Box[str]` son tipos distintos y no se convierten uno en otro. Los métodos de un `impl[T] Box[T]` son funciones genéricas cuyos parámetros de tipo salen del receptor, y un método puede sumar los suyos (`fn map[U](self, f: fn(T) -> U) -> Box[U]`). Las restricciones van en el `impl` (`impl[T: Ord] Tree[T]`), no en el tipo. Un trait se puede implementar para un tipo genérico (`impl[T: Show] Show for Box[T]`), y entonces `Box[int]` cumple `Show` solo si `int` lo cumple.

Cuando los valores no alcanzan para decidir un argumento de tipo, lo decide el tipo esperado: `let s: Stack[str] = Stack.new()`, `let b: Box[u8] = Box(v: 200)`. También se puede escribir: `Stack[int].new()`, `Tree[int].Leaf`. Un valor al que le falta decidir un argumento (`Tree.Leaf` solo) se puede usar donde el contexto lo fija, pero no se puede guardar en una variable `var` sin tipo, porque la variable podría recibir después valores de tipos distintos.

### Módulos y librería estándar

Un archivo es un módulo. `import geo.shapes` carga `geo/shapes.mou`, buscado junto al archivo que se le pasa al compilador, y sus nombres se usan con el último tramo del camino (`shapes.Point`, `shapes.dist(a, b)`) o con el alias de `import geo.shapes as g`. Cada módulo tiene sus propios nombres: dos módulos pueden declarar un `Point` distinto. Desde otro módulo solo se ve lo declarado `pub`. Los imports circulares funcionan. Un error o un panic se informa en el archivo del módulo donde está.

`import std.nombre` carga un módulo de la librería estándar, que viaja dentro del compilador. Salvo `std.io` y `std.raw`, está escrita en el propio lenguaje, en el directorio `std/`:

| Módulo | Contenido |
|---|---|
| `std.io` | Del compilador: `io.print`, `io.eprint` (a la salida de errores), `io.read_line` (devuelve `None` al terminar la entrada), `io.read_file`, `io.write_file`, `io.args` (los argumentos del programa) e `io.Error` |
| `std.raw` | Del compilador, pensado como soporte del resto de la librería: `raw.concat`, `raw.from_chars`, `raw.hash` (FNV-1a de un string), `raw.mix` (dispersa los bits de un entero) |
| `std.hash` | `Table[K, V]` con `K: Key`: tabla hash de direccionamiento abierto con sondeo lineal, ocupada como mucho en tres cuartos; borrar cierra el hueco sin dejar marcas. El trait `Key` (`hash` y `same`) está implementado para `int`, `str`, `char` y `bool`, y cualquier tipo propio puede implementarlo |
| `std.cmp` | `Order`, el trait `Ord` con `impl` para `int`, `f64`, `str` y `char`, `min`, `max`, `sort` (merge sort estable) |
| `std.list` | `List[T]`: `push`, `pop`, `get`, `at`, `set`, `insert`, `remove`, `map`, `filter`, `fold`, `any`, `every`, `reverse`, y con `T: Ord` también `sort`, `index_of`, `contains`, `smallest`, `largest` |
| `std.map` | `Map[K, V]` con `K: Ord`, guardado como arrays ordenados: buscar es logarítmico, insertar y borrar son lineales |
| `std.math` | `abs`, `fabs`, `min`, `max`, `clamp`, `pow`, `gcd` |
| `std.text` | `chars`, `count`, `split`, `words`, `lines`, `join`, `trim`, `find`, `contains`, `starts_with`, `ends_with`, `repeat`, `parse_int`, `concat`, y `Builder` (`add`, `add_char`, `line`, `build`) para armar un string largo en tiempo lineal |

Límites de esta primera librería: concatenar con `"{a}{b}"` dentro de un bucle sigue siendo cuadrático (para eso está `text.Builder`); los archivos se leen y escriben enteros y solo como texto UTF-8 (no hay lectura por partes, bytes sueltos ni directorios); `Table` no se puede recorrer con `for` (se piden `keys()` o `values()`); no hay raíz cuadrada ni conversión de mayúsculas; `Ord` y `Key` no están implementados para los demás tipos enteros.

### Tests y `assert`

`assert(condición)` y `assert(condición, mensaje)` detienen el programa con un panic cuando la condición es falsa; el mensaje solo se calcula si falla. `mou test archivo.mou` corre, en el intérprete, cada función sin parámetros del archivo cuyo nombre empieza con `test_`. Un test falla si entra en panic o si devuelve un error; lo que imprime solo se muestra cuando falla. El comando termina con código 1 si alguno falló. Límites: no corre los tests de los módulos importados, no filtra por nombre y no los ejecuta como binario nativo.

### Referencias de retorno: cómo están hechas

Una función `-> ref mut T` se compila dos veces. La primera forma es la función de siempre, que devuelve el valor; es la que se usa al leer, al guardarla como valor función y a través de `dyn`. La segunda devuelve el camino hasta el resultado: una lista de pasos (campo o índice) desde el primer parámetro. Cuando el resultado de la llamada se escribe (`v.at(i) = x`, `v.at(i).n += 1`, `f(mut v.at(i))`, `v.row(i).push(x)`), el que llama usa la segunda forma y aplica los pasos sobre su propia variable. Así el cuerpo corre una sola vez, la escritura se hace en el lugar y no se copia el contenedor. Los pasos se verifican al aplicarse, igual que un índice común: un camino nunca apunta a memoria inválida.

Qué se verifica:
- `ref mut` exige que `self`, o el primer parámetro, sea `mut`.
- El resultado tiene que ser ese parámetro, un campo o un elemento suyo (no un rango), u otra llamada `ref` sobre él. No vale una variable local, un valor temporal ni otro parámetro. Un `if` con `else` vale si las dos ramas cumplen.
- No se usa `return` ni se puede fallar con `!`.
- Solo se puede escribir en el resultado de un `ref mut`, y la variable de origen tiene que ser mutable.
- El resultado cuenta como un acceso a toda la variable de origen: `swap(mut v.at(0), mut v.at(1))` se rechaza, igual que modificarla dentro de un `for` que la recorre.
- Un `impl` tiene que declarar el método con el mismo `ref` que su trait. Funciona a través de cotas (`T: Trait`) y de métodos por defecto.

Límites: `ref T` de solo lectura hoy se comporta como devolver el valor (el runtime ya comparte sin copiar); el resultado de un `ref mut` no se puede escribir a través de `dyn` ni de un valor función; no hay `match` como última expresión; no hay regiones explícitas ni referencias guardadas en structs.

### Efectos y perfiles: qué se verifica

El análisis está en `src/effects.rs` y trabaja sobre el MIR, después de monomorfizar. Para cada función infiere cuatro efectos: `alloc`, `panic`, `io` y `unsafe` (llega a una función de C). Una función tiene los efectos de sus operaciones y los de todo lo que llama. `mou effects archivo.mou` los imprime.

`@noalloc`, `@nopanic` y `@noio` sobre una función prohíben un efecto. Cada operación que lo tendría es un error en su lugar exacto; si viene de una llamada, el mensaje sigue la cadena hasta la operación que lo causa. Una anotación desconocida es un error. Un módulo que empieza con `@profile(system)` prohíbe `alloc` en todas sus funciones, porque todavía no hay un contexto que le pase un alocador.

Qué cuenta como `panic`: `+`, `-` y `*` enteros que pueden salirse del tipo, división y resto por un divisor que puede ser cero, desplazamientos fuera de rango, `as` y literales que pueden no entrar en el tipo, índices y rangos, `panic`, un `assert` que puede fallar, una condición `where` del llamado que no se prueba en la llamada, y la recursión (la profundidad de llamadas tiene un límite). `+%`, `+|`, los flotantes, `get`, `pop` y las comparaciones no tienen el efecto.

Una operación que solo falla para algunos valores no cuenta cuando se prueba que los valores son buenos. La prueba es un análisis de flujo de datos hacia adelante que sabe, en cada punto, un rango para cada entero, qué enteros están por debajo del largo de qué arrays y un mínimo para el largo de cada array. Aprende de las condiciones de `if`, `while`, `for`, `assert` y `where`, incluso combinadas con `&&`, `||` y `!` (que por eso se bajan al MIR como saltos). Con eso se prueban, entre otros: `xs[i]` dentro de `for i in 0..xs.len` o después de `if i < xs.len`, `xs[0]` con `where xs.len > 0`, `xs[xs.len - 1]` y `xs[i % xs.len]` cuando el array no está vacío, `a / b` después de `if b != 0`, y `n as u8` después de comparar `n` con los límites. Lo que se sabe de un array se olvida cuando su largo puede cambiar (`push`, `pop`, asignarlo, pasarlo como `mut`); escribir un elemento no lo cambia.

Las condiciones `where` se verifican como dice la sección 6: en la llamada, con lo que ahí se sabe de los argumentos. Si se prueban, la llamada no tiene el efecto `panic` por ellas; si no, lo tiene.

Qué cuenta como `alloc`: crear un array con elementos, `[v; n]`, `push`, copiar una parte de un array, armar un string con interpolación, `chars`, una closure que captura, un valor `dyn`, un valor de un tipo recursivo, las funciones nativas que devuelven strings o arrays, y copiar a otra variable o campo un valor que contiene un array (`let ys = xs`). Crear un struct, un enum no recursivo, una tupla o un opcional no cuenta.

Límites conocidos:

- `@noalloc` habla del lenguaje, no del binario de hoy: el emisor de C todavía guarda los structs sueltos, los enums y los enteros de 128 bits en bloques de memoria, así que una función `@noalloc` que crea un struct aloca en el binario actual. Deja de pasar cuando esos tipos tengan layout propio.
- Una llamada a un valor función o a un método de un valor `dyn` tiene los tres efectos, porque no se sabe a qué llama. Los tipos de función todavía no llevan efectos.
- Un literal de array cuenta como `alloc` aunque su tipo sea de largo fijo.
- La prueba no relaciona dos variables entre sí salvo con el largo de un array: no prueba `xs[i]` con `i < n` y `n <= xs.len`, ni un bucle que cuenta hacia atrás, ni índices en arrays anidados (`m[i][j]`), ni búsqueda binaria. En esos casos hay que usar `get` o comparar con el largo.
- Toda recursión cuenta como `panic`, aunque termine.
- Quedarse sin memoria aborta el programa y no es el efecto `panic`.
- Falta el efecto `block` (no hay todavía operaciones que lo tengan), una anotación que prohíba `unsafe`, el contexto (`with alloc = arena`) y las demás reglas de los perfiles: tipos obligatorios en firmas, copias explícitas en `app`, y regiones solo en `system` (`unsafe` solo en `system` ya se verifica).

### Ejecución en compilación: qué hay

Las constantes globales (`let TABLE: [u32] = crc_table()`) y las expresiones `comptime e` se calculan mientras se compila, con el mismo intérprete de MIR que usa `mou run`. En el programa queda el valor: un número o un string se escribe donde se usa, y un array, struct, enum u opcional se arma una vez al arrancar a partir de sus datos, sin volver a correr la función que lo calculó.

- Un panic al calcular (división por cero, índice fuera de rango, overflow) es un error de compilación, en el lugar de la operación.
- No hay entrada ni salida: una constante que llega a `io` es un error, y el mensaje dice por qué función llega. Eso lo decide el análisis de efectos.
- Hay un límite de pasos (100 millones de bloques para todo el programa), para que un bucle infinito sea un error y no un compilador colgado.
- `comptime e` no puede usar variables de la función donde está, porque no tienen valor al compilar. Puede usar constantes globales y llamar a cualquier función.

Límites conocidos: `comptime` toma toda la expresión que sigue (`comptime a + b` calcula la suma), así que para mezclar con algo que se calcula al ejecutar van paréntesis: `(comptime a) + b`. Una constante que guarda una closure se sigue calculando al arrancar. Faltan los parámetros `comptime`, los tipos como valores (`fn Matrix(comptime T: type) -> type`) y la reflexión con `info(T)`.

### Funciones de C: qué hay

Un módulo declara las funciones de un header que usa, y las llama con el nombre del import:

```mou
@profile(system)
import c "math.h" as m {
    fn sqrt(x: f64) -> f64
    fn pow(x: f64, y: f64) -> f64
}

pub fn root(x: f64) -> f64 { unsafe { m.sqrt(x) } }
```

- El compilador no lee el header: confía en la declaración. Por eso la llamada solo se acepta dentro de `unsafe { }`, y `unsafe` solo se acepta en un módulo `system`. Lo normal es un módulo `system` chico que envuelve las funciones de C en funciones seguras, y el resto del programa usa esas.
- Parámetros y resultado: enteros de hasta 64 bits, `f32`, `f64` y `bool`, o ningún resultado. Los argumentos se verifican contra la declaración como en cualquier llamada.
- El binario incluye el header y llama a la función directamente. `mou build archivo.mou -l sqlite3` enlaza una librería.
- Una función que llega a una de C tiene el efecto `unsafe`, que `mou effects` muestra. De lo que hace la función de C (alocar, entrada y salida) responde quien escribió el `unsafe`.
- El intérprete no puede llamar a C: `mou run`, `mou test`, el `repl` y `comptime` se detienen con un panic que lo dice cuando llegan a una. Un programa que usa C se prueba compilado.

Faltan: leer el header en vez de declarar a mano, strings, punteros, structs con layout de C, funciones de mou como callbacks, y `import cpp` e `import py`.

### Facilidad de uso: lo que el compilador deduce

Tres reglas sacan texto que no agrega seguridad:

- `n as char` da el carácter con ese código, y hace panic si el número no es el código de ninguno (negativo, mayor que 0x10FFFF, o entre 0xD800 y 0xDFFF). `c as int` da el código. `text.parse_float(s)` lee un número con fracción de un texto (`-12.5e3`) y da `None` si el texto es otra cosa.
- Un entero se convierte solo a un tipo que contiene todos sus valores: `u8` a `int`, `i32` a `i64`, `u32` a `i64`. Vale al asignar, al pasar argumentos, al devolver y al mezclar tipos en una operación (`u8 + int` se calcula como `int`). Cuando se puede perder información (`int` a `u8`, con signo a sin signo) sigue haciendo falta `as`, que comprueba el valor. `usize` e `isize` no se convierten solos, porque su tamaño depende de la máquina.
- `var xs = [1, 2]` es un array que puede crecer. Con `let` sigue siendo de largo fijo.
- Un número escrito que no entra en el tipo que le toca es un error de compilación (`let b: u8 = 300`, `x < 1000` con `x: u8`), no un panic al ejecutar.

Lo que se sigue escribiendo a propósito: `mut` y `own` en la llamada, `?` en lo que puede fallar, y `unsafe` alrededor de C.

### Fuzzing diferencial

`mou fuzz [cantidad] [semilla] [--sanitize]` escribe programas al azar, los corre en el intérprete y como binario nativo, y avisa cuando no coinciden en lo que imprimen o en cómo terminan. También avisa si el compilador rechaza un programa (el generador solo escribe programas válidos), si el compilador de C lo rechaza, y con `--sanitize` si el binario tiene comportamiento indefinido. Cada caso que falla queda guardado con su semilla.

El generador cubre: enteros de siete tipos con todas las operaciones (las que comprueban, las que dan la vuelta y las que saturan), desplazamientos, conversiones con `as` y sin él, booleanos con `&&`, `||` y `!`, `f64`, variables, `if`, `for`, `while`, arrays de `int` con `push` e índices, structs con campos de distintos tipos y llamadas entre funciones. No cubre todavía: enums y `match`, opcionales, errores, strings, genéricos, traits, closures, `ref`, `comptime` ni módulos.

Lo que encontró hasta ahora: un literal más grande que su tipo dentro de una operación que satura o da la vuelta se calculaba distinto en el intérprete y en el binario. Ahora es un error de compilación.

### El compilador escrito en Mou

Para construirlo sin Rust: `./bootstrap.sh`. Compila la semilla (`seed/mouc.c.gz`, el compilador ya pasado a C por sí mismo), con ella compila los fuentes de `compiler/`, compila el resultado y comprueba que ese segundo compilador vuelve a escribir el mismo C. Deja el binario en `./mouc`. `./reseed.sh` vuelve a escribir la semilla; hace falta cuando el compilador empieza a usar algo del lenguaje que la semilla anterior no entiende.

La meta es que Mou se compile a sí mismo. Se hace por etapas, y cada etapa se da por buena cuando produce exactamente lo mismo que su par de Rust sobre todos los archivos `.mou` del proyecto.

| Etapa | Estado |
|---|---|
| Lexer | Hecho: `compiler/lexer.mou`, unas 420 líneas |
| Parser | Hecho: `compiler/parser.mou`, unas 1600 líneas con el árbol y su volcado |
| Cargador de módulos | Hecho: `compiler/load.mou`, unas 170 líneas |
| Verificador de tipos | Hecho: `compiler/types.mou` y `compiler/check.mou`. Declaraciones, constantes, cuerpos de funciones y métodos, patrones y exhaustividad, llamadas, genéricos con sus instancias, traits, `dyn`, resultados por referencia y reglas de exclusividad. Se verifica a sí mismo: da exactamente lo mismo que el de Rust para sus propias 15.000 expresiones |
| Bajada a MIR | Hecho: `compiler/mir.mou`. Cada función queda como bloques de sentencias simples: patrones, bucles, `defer`, closures y capturas, `comptime`, conversiones, llamadas con argumentos `mut` que vuelven a su lugar, funciones gemelas para los resultados `ref mut`, `dyn`. Da el mismo MIR que el de Rust para todo el proyecto, incluidas sus propias 578 funciones |
| Ownership | Hecho: `compiler/moves.mou`. Variables leídas sin valor, usadas después de moverlas con `own`, o inmutables asignadas dos veces |
| Efectos | Hecho: `compiler/effects.mou`. Infiere `alloc`, `panic`, `io` y `unsafe`, prueba rangos de enteros e índices con el mismo análisis de flujo y da los mismos errores para `@noalloc`, `@nopanic`, `@noio` y los módulos `system`. Es lento sobre programas grandes (44 s sobre el propio compilador contra menos de 1 s del de Rust), porque cada rango es hoy un objeto en el heap |
| `comptime` | Hecho: `compiler/consts.mou`, con un intérprete de MIR. Las constantes y las expresiones `comptime` quedan calculadas en el binario, y un pánico o entrada y salida al compilar son errores. Las operaciones con `f32` las deja para el arranque del programa en vez de calcularlas. No apareció ningún caso en el proyecto |
| Emisor de C | Hecho: `compiler/cgen.mou`. Escribe el mismo archivo C que el de Rust, byte por byte, para todo el proyecto |
| Se compila a sí mismo | **Hecho**. `mouc`, compilado por el compilador de Rust, compila `compiler/main.mou`; el compilador que sale de ahí vuelve a compilar `compiler/main.mou` y escribe exactamente el mismo C (unas 330.000 líneas con el runtime), que además es el que escribe el compilador de Rust |

`mou build compiler/main.mou -o mouc` compila el compilador (con `CFLAGS=-O1` el compilador de C tarda mucho menos). `./mouc c archivo.mou std src/runtime.c` escribe el C del programa, el mismo que `mou c archivo.mou` (`mou rawc` da lo mismo con los errores uno por línea, que es como los imprime `mouc`). `./mouc lex archivo.mou` imprime los tokens, `./mouc tree archivo.mou` el árbol sintáctico, `./mouc modules archivo.mou` el programa entero con todo lo que importa, `./mouc decls archivo.mou` lo que dicen sus declaraciones `./mouc types archivo.mou` lo que el verificador averigua de cada expresión y `./mouc lowered archivo.mou` el programa bajado a MIR; `mou lex`, `mou tree`, `mou modules`, `mou decls`, `mou types` y `mou lowered` imprimen lo del compilador de Rust en el mismo formato, con la posición de cada expresión y de cada declaración. El compilador en Mou ya se carga a sí mismo y verifica sus propias declaraciones: siete módulos. La librería estándar la lee de la carpeta `std` (o de la que se le pase), porque todavía no puede llevarla adentro como el de Rust.

Cómo se comprobó: un test compila `mouc` como programa nativo y corre cada etapa (tokens, árbol, módulos, declaraciones, tipos, MIR y C) sobre los 76 archivos `.mou` del proyecto, incluidos ellos mismos, y exige que tokens, árbol y módulos coincidan con los de Rust. El cargador se probó además con imports en círculo, módulos que no existen, módulos con errores de sintaxis e imports de C. Las funciones de tipos (`join`, `assignable`, `same`, `subst`, `bind` y las demás) se corren sobre una lista fija de 46 tipos, de a pares, en los dos compiladores: `mou typetest` y `./mouc typetest` imprimen las mismas 2419 líneas. La pasada de declaraciones se compara igual: `mou decls` y `./mouc decls` imprimen la misma tabla (nombres, campos, firmas, métodos, implementaciones) para todos los archivos del proyecto, y los mismos errores con las mismas palabras para los programas de `tests/decls`, que tienen a propósito unos 60 errores de declaración. La pasada de cuerpos se compara con `mou types` y `./mouc types`: para cada expresión del programa, y para cada instancia de cada función genérica, el tipo, el tipo al que se convierte donde se usa y a qué se refiere (variable, campo, función, método, instancia). Coinciden en todos los archivos del proyecto, y en los programas de `tests/types`, que tienen a propósito unos 150 errores de tipos, de mutabilidad, de modos, de patrones y de genéricos, los dos dan los mismos mensajes en las mismas posiciones. La bajada a MIR se compara con `mou lowered` y `./mouc lowered`: cada función con sus bloques, sentencias, saltos y posiciones, más las formas de structs y enums. Coincide en todo el proyecto y en `tests/lower`. El C se compara entero, y al final el test hace el ciclo completo: `mouc` se compila a sí mismo, el resultado se compila con el compilador de C y ese segundo compilador tiene que escribir otra vez el mismo C. Sin compilador de C a mano, las etapas corren en el intérprete y el C no se compara. A mano se comprobó lo mismo con el binario nativo, y además con 26 textos con errores de sintaxis o construcciones raras: los dos parsers dan el mismo árbol o el mismo error en la misma posición.

Velocidad: `mouc` tarda 3,6 s en compilarse a sí mismo (unas 12.000 líneas de Mou, con `cc -O1`); el de Rust, 0,3 s. La diferencia está en cómo se guardan hoy los valores: todo lo que no es un número, un booleano o un array de ellos lleva etiqueta y cuenta de referencias, y cada struct es un bloque aparte. El análisis de efectos es el caso más caro, porque guarda un rango por variable y por bloque: 44 s sobre el propio compilador. El arreglo de fondo es el layout propio de structs y enums, pendiente en M4.

Diferencia conocida: el lexer de Mou solo acepta letras ASCII en los identificadores, y el parser solo mira mayúsculas ASCII para decidir si un nombre en un patrón es un constructor, porque los caracteres todavía no tienen métodos como `is_alphabetic`. Los de Rust aceptan cualquier letra Unicode.

Lo que enseñó escribirlos: una llave suelta dentro de un string (`"{"`) abre una interpolación y hay que escribirla doble (`"{{"`); el error de ese caso ahora lo dice en el lugar. Los nombres de variables no pueden ser palabras clave (`region`, `type`, `scope`). Sin closures que modifiquen lo que capturan, las listas del parser se escriben como bucles en vez de pasar una función. Fuera de eso, el parser compiló sin tocar el lenguaje.

El compilador de Rust tiene unas 17 000 líneas. Lexer y parser son cerca del 10 %: el verificador de tipos solo tiene más de 4 000. Para que el resto sea viable hace falta antes el layout propio de structs y enums, porque un compilador mueve muchísimos de esos valores.

### Formateador y sesión interactiva

`mou fmt archivo.mou` le da al archivo el formato estándar; con `--check` solo informa si ya lo tiene (código de salida 1 si no). Trabaja sobre el texto y no sobre el árbol, así que los comentarios quedan donde están. Corrige la sangría (cuatro espacios por nivel de llave, paréntesis o corchete; uno más en las líneas que continúan la anterior), los espacios dentro de la línea (alrededor de `=`, `->`, `=>` y de los operadores que siempre son binarios; después de `,` y `:`; ninguno pegado por dentro a paréntesis y corchetes) y las líneas en blanco (nunca dos seguidas, ninguna al principio del archivo ni de un bloque). No toca strings ni comentarios, y nunca pasa algo de una línea a otra: dónde termina una línea lo decide quien escribe, y en este lenguaje tiene significado. Antes de escribir compara los tokens del resultado con los del original y se niega si difieren. Límites: no parte líneas largas, no alinea columnas (une a un espacio lo que estaba alineado a mano) y deja como estén los espacios alrededor de `-`, `&`, `|`, `!`, `.` y `..`, que pueden ser unarios o ir pegados.

`mou repl` abre una sesión interactiva. La sesión es un programa que crece: una declaración (`fn`, `struct`, `enum`, `trait`, `impl`, `import`) se agrega al programa, una sentencia se agrega a su `main`, y una expresión con valor además lo muestra. Una entrada que deja un paréntesis, corchete o llave abierto sigue en la línea siguiente. Si una entrada no compila o termina en panic, la sesión queda como estaba. Después de cada entrada el programa entero se verifica y se corre de nuevo desde el principio, y se muestra solo lo que imprime de más: eso mantiene la sesión tan estricta como un archivo, al precio de repetir las sentencias anteriores (lo que imprimen se oculta, pero lo que leen o escriben vuelve a pasar). Límites: no tiene edición de línea ni historial propios, los errores se muestran sin posición, y no se puede redefinir una función ya declarada (hay que usar `:reset`).

### Ownership y exclusividad: qué se verifica

En la llamada (sobre el árbol de sintaxis, en `src/check.rs`):
- Dos argumentos de la misma llamada no pueden acceder al mismo valor si uno es `mut`. El receptor de un método cuenta como argumento. Los préstamos son por campo: `swap(mut p.x, mut p.y)` vale, `swap(mut p.x, mut p.x)` no. Dos índices solo se consideran distintos si ambos son literales.
- Un argumento escalar (entero, flotante, `bool`, `char`) se copia antes de la llamada y no cuenta como acceso: `addn(mut p, p.y)` vale.
- Un argumento no puede modificar lo que otro argumento anterior de la misma llamada tomó prestado: `addn(mut p, bump(mut p.x))` no vale.
- El cuerpo de un `for` no puede modificar lo que el `for` recorre.
- `own x` exige una variable entera (no un campo), que no sea un parámetro prestado ni una captura de closure.

En el flujo de control (sobre el MIR, en `src/own.rs`):
- Una variable declarada sin valor (`let x: int`) no se puede leer hasta que todos los caminos le hayan dado uno, y si es `let` solo se le puede asignar una vez.
- Después de `f(own x)`, `x` no se puede usar hasta que se le asigne de nuevo. Vale también a través de ramas y de vueltas de bucle.

Pasar `x` sin `own` a un parámetro `own` copia el valor. Falta decidir si eso debe ser un error fuera del último uso, como dice la tabla de la sección 4.

No se verifica todavía: `ref` como tipo de retorno, las regiones (`ref[r]`), ni nada sobre `unsafe`.

### El emisor de C

`mou build archivo.mou` genera un único archivo C (visible con `mou c`) y llama al compilador de C (`cc`, o el de la variable `CC`). Necesita `__int128`, o sea GCC o Clang.

Los valores compuestos (structs, enums, arrays, strings, opcionales) se representan igual que en el intérprete: etiquetados y con conteo de referencias. Los enteros de hasta 64 bits, los booleanos y los `f64`, no: el emisor mira cada variable local y, si su tipo es uno de esos (o si solo guarda enteros de un mismo tipo, solo booleanos o solo `f64`), la declara como una variable de C común. La aritmética sobre ellas se escribe en línea con detección de overflow del compilador (`__builtin_add_overflow` y parientes); el camino lento, que es el que arma el mensaje del panic, solo corre cuando hay overflow, división por cero o por `-1`, o una operación menos común (desplazamientos, envolventes, saturantes). Una función cuyos parámetros y resultado son todos enteros o booleanos, y que no modifica sus parámetros, se llama directamente con valores de máquina; conserva además una entrada con valores etiquetados para cuando se la usa como valor.

Los arrays cuyos elementos son todos booleanos, enteros que entran en 64 bits, `f64` o `char` guardan los elementos sin etiqueta: 1, 8, 8 y 4 bytes cada uno en vez de 32. La decisión es del runtime y es por array: se toma al crearlo o al recibir el primer elemento, y si después llega un valor que no entra (un `i128` grande, por ejemplo) el array pasa a la forma etiquetada sin que el programa lo note. El código generado lee y escribe esos elementos en línea, con la comprobación de rango y la de copia al escribir. Los campos de structs y los elementos de arrays de structs se alcanzan con punteros en línea, pero siguen siendo valores etiquetados en bloques aparte.

`for i in a..b` sobre enteros cuenta en dos variables de máquina y no construye el valor rango. La posición del programa (línea, columna y función) se guarda en una sola palabra, así que entrar y salir de una función no escribe nada más.

Cómo se comprueba: un test compila los ejemplos y 42 programas de casos límite (`tests/native/`: overflow en cada ancho, división, restos, conversiones, sin signo cerca de 2^64, parámetros `mut` enteros, rangos, recursión, arrays empaquetados que cambian de forma) y exige que el binario imprima lo mismo y salga con el mismo código que el intérprete, panics incluidos. Los binarios de los ejemplos y de varios casos límite pasan AddressSanitizer y UBSan sin errores ni fugas.

Mediciones en esta máquina, con GCC `-O2`, contra el mismo programa escrito a mano en C:

| Programa | Mou nativo | C | Nota |
|---|---|---|---|
| `fib(40)` recursivo | 0,36 s | 0,09 s | el C no detecta overflow ni profundidad de pila |
| bucle de 100 millones de sumas, productos y restos | 0,21 s | 0,22 s | el C usa las mismas comprobaciones de overflow |
| criba de 5 millones sobre `[bool]` | 0,04 s | 0,01 s | antes 0,39 s y 160 MB; ahora usa la misma memoria que C |
| Mandelbrot de 600 por 600 con `f64` | 0,05 s | 0,05 s | |
| 40 millones de actualizaciones sobre un array de 1000 structs | 0,16 s | 0,02 s | antes 1,2 s: cada struct era un bloque aparte con campos etiquetados. Queda el costo de comprobar el índice y la forma del array en cada acceso |

Lectura honesta: los bucles con enteros, booleanos y `f64`, y los arrays de esos tipos, están entre la par y 4 veces de C. Las llamadas recursivas están a 4 veces. Los arrays de structs de escalares quedaron a unas 8 veces en el caso medido. Los structs sueltos, los structs con campos que no son escalares, los strings y los enums siguen lejos, porque esos valores todavía son bloques etiquetados con conteo de referencias (no hay medición de esos casos). La meta de velocidad está cumplida solo para código escalar. Lo que falta pide llevar los tipos completos al MIR.

Arrays de structs en línea: un array cuyos elementos son structs con todos los campos `bool`, `char`, `f64` o enteros de hasta 64 bits con signo (o 32 sin signo) guarda los campos uno al lado del otro, ocho bytes cada uno, sin un bloque por elemento. `ps[i].x` lee y escribe ahí directamente. Pedir el elemento entero (`ps[i]`, `for p in ps`, `pop`) arma un struct suelto, y guardarlo lo vuelve a desarmar. El array vuelve a la forma general si hace falta la dirección de un elemento, cosa que hoy pasa al escribir a través de un `ref mut` que llega hasta él. Los structs genéricos, los que tienen `u64`, `i128`, `f32`, strings, arrays u otros structs adentro siguen como antes.

La recursión demasiado profunda se informa como panic midiendo el uso de la pila (6 MB, 700 KB en Windows).


### Qué verifica hoy el verificador de tipos

Verifica: primitivos, structs, enums, tuplas, slices, opcionales, resultados con error, funciones, genéricos con restricciones en funciones, structs, enums, `impl` y alias (`fn max[T: Ord]`, `impl[T: Ord] Tree[T]`), traits y la conformidad de cada `impl Trait for Tipo`, nombres de otros módulos y su visibilidad (`pub`), closures, mutabilidad (`let`/`var`, modos `mut`/`own` en la llamada), `if`/`match` con exhaustividad de un nivel, bucles, `?`, `catch` y resultados ignorados.

Los traits admiten supertraits (`trait Shape: Named`) y métodos con cuerpo. Un método con cuerpo se verifica una vez en el trait, para cualquier `Self` que lo implemente, y se compila una vez por cada tipo que no lo redefine. Un `impl` de un trait exige que el tipo implemente también lo que el trait requiere.

`dyn Trait` es el tipo de un valor de cualquier tipo que implemente el trait. La conversión es explícita, `x as dyn Shape`, de acuerdo con la regla de que el despacho dinámico se ve en el código; no hay conversión implícita. El valor viaja junto con sus métodos (los del trait y los de lo que el trait requiere), se copia como cualquier otro valor y se muestra como el valor que guarda. Sobre un `dyn` no se pueden llamar los métodos sin `self`, los que nombran `Self` en parámetros o resultado ni los que tienen parámetros de tipo propios, porque necesitan el tipo real. `dyn A + B` no existe: se declara un trait que requiera a los dos.

Los traits pueden tener parámetros de tipo: `trait Into[T]`, `impl Into[str] for int`, `fn f[A: Into[B], B]`. Un parámetro que ningún argumento menciona se deduce de la cota: con `A = int`, `B` es lo que `int` declara en su `impl`. `dyn Into[str]` es un tipo distinto de `dyn Into[int]`.

Un tipo puede implementar un trait genérico varias veces, con argumentos distintos: `impl Into[str] for Meters` e `impl Into[int] for Meters`, o `impl From[int]` e `impl From[str]`. Qué método se llama lo decide, en este orden: el tipo que la llamada tiene que producir (`let n: int = m.into()`, o el tipo del parámetro al que se pasa), las cotas de la función genérica en la que está (dentro de `fn f[A: Into[str]]`, `a.into()` es el de `Into[str]`) y los tipos de los argumentos (`Meters.from(7)` contra `Meters.from("hola")`). Si nada decide, es un error que pide darle un tipo al resultado. Por dentro, los métodos de un `impl` de un trait genérico se guardan con el trait en el nombre, y el nombre simple lleva a la lista de candidatos. Límite: para implementar un trait por segunda vez, los argumentos de cada `impl` no pueden tener parámetros de tipo (`impl[T] Into[T] for W[T]` junto con otro `impl Into[...] for W[T]` se rechaza).

Un método de un trait puede tener parámetros de tipo propios: `fn map[U](self, f: fn(int) -> U) -> U`, también con cuerpo por defecto. El `impl` los puede nombrar distinto, pero tienen que ser la misma cantidad y con las mismas cotas. Se deducen de los argumentos y del resultado esperado.

A través de un parámetro de tipo se pueden llamar las funciones sin `self` de sus cotas: con `T: From[int]`, `T.from(n)`.

No verifica todavía (lo informa como "not checked" en vez de aceptarlo en silencio): tipos con parámetros de región o de valor, los imports de C++ y Python, los métodos de tipos primitivos que no sean `push`, `pop` y `get` de `[T]`, closures sin tipos en los parámetros, declaraciones anidadas en bloques, longitud de arrays y regiones explícitas.
