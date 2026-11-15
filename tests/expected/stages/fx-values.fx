fn 0 Sq.area: none
fn 1 Wide.area: panic
  site 10:46 2 `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first
fn 2 apply: panic
  call 13:46 7
  call 13:46 8
  call 13:46 12
fn 3 both: none
  call 16:50 9
fn 4 measure: panic
  call 19:35 0
  call 19:35 1
fn 5 total: none
  call 24:28 7
  call 25:35 7
fn 6 run: none
  call 30:33 14
fn 7 double: none
fn 8 square: panic
  site 33:28 2 `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first
fn 9 add: none
fn 10 shout: io
  site 36:5 4 `io.print` writes to the output
fn 11 main: alloc panic io block
  site 41:17 1 a `dyn` value is created here
  site 42:16 1 a `dyn` value is created here
  site 45:17 1 `spawn` creates the task on the heap
  site 46:20 32 `wait` waits for a task
  site 46:18 1 a string is built here
  site 46:9 4 `io.print` writes to the output
  site 44:5 32 a `scope` waits for its tasks where it ends
  site 46:9 32 a `scope` waits for its tasks where it ends
  site 48:14 1 a string is built here
  site 48:5 4 `io.print` writes to the output
  site 49:59 1 an array is created here
  site 49:55 1 an array is converted here, which makes a new one
  site 49:14 1 a string is built here
  site 49:5 4 `io.print` writes to the output
  call 45:17 13
  call 48:16 2
  call 48:35 2
  call 48:54 2
  call 48:71 3
  call 49:16 4
  call 49:33 4
  call 49:49 5
  call 49:65 6
fn 12 closure: none
fn 13 task: io
  call 45:25 10
fn 14 closure: none
error 19:35 `measure` is `@nopanic`, but the function value or `dyn` method called here can be `Wide.area`, which can panic: `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first (line 10 of its file)
