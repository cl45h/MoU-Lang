fn 0 first: none
  unproved
fn 1 item: none
  unproved
fn 2 twice: panic
  site 6:27 2 `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first
fn 3 say: io
  site 7:23 4 `io.print` writes to the output
fn 4 case0: panic
  site 11:12 2 this index is not proved to be in range: compare it with the length first, or use `get`
  site 11:1 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  call 11:1 2
fn 5 case1: io
  call 16:1 3
fn 6 case2: io
  site 22:1 4 `io.print` writes to the output
fn 7 main: none
error 16:1 `case1` is `@noio`, but it calls `say`, which does input or output: `io.print` writes to the output (line 7 of its file)
error 22:1 `case2` is `@noio`, but `io.print` writes to the output
