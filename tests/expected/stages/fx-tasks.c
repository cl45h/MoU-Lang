error 12:17 `quiet` is `@nopanic`, but it calls `task`, which can panic: it calls `fib`, where this call is recursive, and the nesting of calls has a limit (line 5 of its file)
error 13:17 `quiet` is `@nopanic`, but it calls `task`, which can panic: it calls `fib`, where this call is recursive, and the nesting of calls has a limit (line 5 of its file)
error 21:17 `loud` is `@noio`, but it calls `task`, which does input or output: `io.print` writes to the output (line 21 of its file)
error 30:17 `frugal` is `@noalloc`, but `spawn` creates the task on the heap
error 38:17 `careless` is `@nopanic`, but it calls `task`, which can panic: this index is not proved to be in range: compare it with the length first, or use `get` (line 38 of its file)
