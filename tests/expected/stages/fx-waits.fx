fn 0 magnitude: unsafe
  site 1048582:44 16 the C function `abs` is called here
fn 1 fib: panic
  site 6:27 2 this call is recursive, and the nesting of calls has a limit
  site 6:42 2 this call is recursive, and the nesting of calls has a limit
  call 6:27 1
  call 6:42 1
fn 2 starts: alloc panic block
  site 13:19 1 `spawn` creates the task on the heap
  site 13:19 1 `push` can make the array grow
  site 12:5 32 a `scope` waits for its tasks where it ends
  site 15:5 2 this index is not proved to be in range: compare it with the length first, or use `get`
  call 13:19 9
fn 3 waits: block
  site 19:33 32 `wait` waits for a task
fn 4 reads: alloc io block
  site 23:16 32 `io.read_line` waits for input
  site 23:16 4 `io.read_line` reaches outside the program
  site 23:16 1 `io.read_line` creates its result on the heap
fn 5 calls: panic
  call 32:38 6
fn 6 pure: panic
  call 36:26 1
fn 7 foreign: unsafe
  call 39:29 0
fn 8 main: alloc panic io unsafe block
  site 42:14 1 a string is built here
  site 42:5 4 `io.print` writes to the output
  call 42:22 2
  call 42:16 3
  call 42:34 4
  call 42:44 5
  call 42:58 7
fn 9 task: panic
  call 13:27 1
error 12:5 `starts` is `@noblock`, but a `scope` waits for its tasks where it ends
error 19:33 `waits` is `@noblock`, but `wait` waits for a task
error 23:16 `reads` is `@noblock`, but `io.read_line` waits for input
error 39:29 `foreign` is `@nounsafe`, but it calls `magnitude`, which runs code the compiler does not check: the C function `abs` is called here (line 6 of its file)
