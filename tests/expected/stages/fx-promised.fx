fn 0 double: none
fn 1 risky: panic
  site 4:27 2 this `/` can divide by zero: check the divisor first
fn 2 loud: alloc io
  site 6:14 1 a string is built here
  site 6:5 4 `io.print` writes to the output
fn 3 apply: alloc panic io
  call 11:55 1
  call 11:55 7
  call 11:55 0
  call 11:55 2
  call 11:55 0
fn 4 silent: alloc io
  call 14:62 2
  call 14:62 0
fn 5 pick: none
fn 6 main: alloc panic io
  call 22:5 3
  call 23:5 3
  call 24:5 3
  call 25:5 4
  call 26:5 4
fn 7 closure: panic
  site 23:31 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
error 17:38 a `@nopanic` function is expected here, but `risky` can panic: this `/` can divide by zero: check the divisor first (line 4 of its file)
error 22:11 a `@nopanic` function is expected here, but `risky` can panic: this `/` can divide by zero: check the divisor first (line 4 of its file)
error 23:11 a `@nopanic` function is expected here, but `closure` can panic: `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first (line 23 of its file)
error 25:12 a `@noio` function is expected here, but `loud` does input or output: `io.print` writes to the output (line 6 of its file)
