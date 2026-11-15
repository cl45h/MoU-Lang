fn 0 fib: panic
  site 5:27 2 this call is recursive, and the nesting of calls has a limit
  site 5:42 2 this call is recursive, and the nesting of calls has a limit
  call 5:27 0
  call 5:42 0
fn 1 quiet: alloc panic block
  site 12:17 1 `spawn` creates the task on the heap
  site 13:17 1 `spawn` creates the task on the heap
  site 14:9 32 `wait` waits for a task
  site 14:21 32 `wait` waits for a task
  site 11:5 32 a `scope` waits for its tasks where it ends
  call 12:17 7
  call 13:17 8
fn 2 loud: alloc io block
  site 21:17 1 `spawn` creates the task on the heap
  site 22:9 32 `wait` waits for a task
  site 20:5 32 a `scope` waits for its tasks where it ends
  call 21:17 9
fn 3 frugal: alloc block
  site 30:17 1 `spawn` creates the task on the heap
  site 31:9 32 `wait` waits for a task
  site 29:5 32 a `scope` waits for its tasks where it ends
  call 30:17 10
fn 4 careless: alloc panic block
  site 38:17 1 a closure that captures values is created here
  site 38:17 1 `spawn` creates the task on the heap
  site 39:9 32 `wait` waits for a task
  site 37:5 32 a `scope` waits for its tasks where it ends
  call 38:17 11
fn 5 constant TOTAL: alloc panic block
  call 43:18 1
fn 6 main: alloc panic io block
  site 46:73 1 an array is created here
  site 46:63 1 an array is converted here, which makes a new one
  site 46:14 1 a string is built here
  site 46:5 4 `io.print` writes to the output
  call 46:24 1
  call 46:34 2
  call 46:43 3
  call 46:54 4
fn 7 task: panic
  call 12:25 0
fn 8 task: panic
  call 13:25 0
fn 9 task: io
  site 21:25 4 `io.print` writes to the output
fn 10 task: none
fn 11 task: panic
  site 38:25 2 this index is not proved to be in range: compare it with the length first, or use `get`
error 12:17 `quiet` is `@nopanic`, but it calls `task`, which can panic: it calls `fib`, where this call is recursive, and the nesting of calls has a limit (line 5 of its file)
error 13:17 `quiet` is `@nopanic`, but it calls `task`, which can panic: it calls `fib`, where this call is recursive, and the nesting of calls has a limit (line 5 of its file)
error 21:17 `loud` is `@noio`, but it calls `task`, which does input or output: `io.print` writes to the output (line 21 of its file)
error 30:17 `frugal` is `@noalloc`, but `spawn` creates the task on the heap
error 38:17 `careless` is `@nopanic`, but it calls `task`, which can panic: this index is not proved to be in range: compare it with the length first, or use `get` (line 38 of its file)
