error 12:5 `starts` is `@noblock`, but a `scope` waits for its tasks where it ends
error 19:33 `waits` is `@noblock`, but `wait` waits for a task
error 23:16 `reads` is `@noblock`, but `io.read_line` waits for input
error 39:29 `foreign` is `@nounsafe`, but it calls `magnitude`, which runs code the compiler does not check: the C function `abs` is called here (line 6 of its file)
