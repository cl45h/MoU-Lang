# Mou

[Español](README.md) | English

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
- Networking: `std.net` gives TCP (listen, connect, read and write), UDP and name lookup, written in Mou on top of C sockets. A server handles each connection in a task. Linux, IPv4 and compiled programs only.
- A standard library written in Mou: lists, maps, hash tables, text with Unicode, math, files and directories (`std.fs`), and `std.pool` with `Pool[T]` and `Handle[T]` for graphs and cyclic structures.
- Python inside the program with `std.py`: `py.module("math")?.method("sqrt", [py.from_float(2.0)])?`. It is a library on top of the C interface of CPython; exceptions arrive as errors.
- Integer conversions that never lose anything silently: `as` keeps the value or panics, and `u8.truncate(x)`, `u8.saturate(x)` and `u8.try_from(x)` say what to do when it does not fit.
- Two ways to run: an interpreter (`mou run`) and native binaries through C (`mou build`).
- Tools: `mou test` (with a name filter, `--all` and `--native`), `mou fmt`, `mou repl`, `mou effects`, `mou fuzz` and `mou bind` (writes the declarations of a C header).
- The compiler written in Mou (`compiler/`): lexer, parser, module loader, type checker, lowering to MIR, ownership, effects, compile-time constants and C emitter. It gives the same output as the Rust one at every stage and **it compiles itself**: the resulting compiler writes exactly the same C again.

## What is missing

The core of the language is complete and the compiler compiles itself. What follows is designed in [DESIGN.md](DESIGN.md) but not built; each item is a large job on its own:

- Performance on par with C: today it matches C in loops, numeric code, and structs (nested ones too), tuples, optionals and enums of numbers, `bool` and `char`, which already live without a memory block. It is still several times slower with strings and with structs and enums that hold strings, arrays or optionals, because those still go in tagged memory blocks.
- References stored in structs, and explicit regions.
- Input and output that does not wait when writing, IPv6 and TLS: today several connections at once are handled with one task (one thread) per connection, or from a single thread with `net.Server`, which hands over what happens as events (`Open`, `Data`, `Done`); Linux and IPv4 only. Cancellable tasks, channels with `Chan[T]` and shared state with `Mutex[T]` already work.
- The rest of the context: replacing files, arguments and environment too. `with alloc = arena`, `with io = lines` (captures what is printed) and `with input = lines` (provides what is read) are already there, and function types already carry written effects (`fn(int) -> int @nopanic`).
- Functions that return a type (`fn Matrix(comptime T: type) -> type`); `comptime` parameters for integers and types (`empty(int)`), fixed-length arrays `[T; N]` and reflection with `meta.info(T)` are already there.
- An LLVM backend of its own, SIMD and `soa`: today LLVM is used through Clang (`CC=clang mou build`), which compiles the same C. `import cpp`, and the language form of `import py` (Python is already usable with `std.py`).

The details, with the known limits of each part, are in [DESIGN.md](DESIGN.md). The grammar is in [GRAMMAR.md](GRAMMAR.md). Both documents are in Spanish.

## Build and test

There are two paths, and they give the same compiler.

With only a C compiler (`cc`), starting from the compiler written in Mou:

```sh
./bootstrap.sh
./mouc c examples/tour.mou std src/runtime.c > tour.c && cc -O2 -o tour tour.c -lm && ./tour
```

With Rust, which also brings the interpreter (`mou run`), the interactive session, the formatter and the tests:

```sh
cargo build --release
./target/release/mou run examples/tour.mou
./target/release/mou build examples/tour.mou -o tour && ./tour
cargo test --release
```

## Name

Mou, or mou-lang when searching for it. The command is `mou` and files end in `.mou`.

## License

Mou is distributed under two licenses, at the user's choice: [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE).
