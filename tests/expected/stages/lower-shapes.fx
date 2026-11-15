fn 0 Sq.area: none
fn 1 constant PI: none
fn 2 constant BIG: none
fn 3 constant NAMES: alloc
  site 11:26 1 an array is created here
fn 4 grow: alloc panic
  site 14:13 1 `push` can make the array grow
  site 15:5 2 this index is not proved to be in range: compare it with the length first, or use `get`
  unproved
fn 5 sum: panic
  site 23:28 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 23:28 2 this call is recursive, and the nesting of calls has a limit
  site 23:37 2 this call is recursive, and the nesting of calls has a limit
  call 23:28 5
  call 23:37 5
fn 6 shapes: none
  call 29:29 9
  call 30:11 7
fn 7 note: none
fn 8 main: alloc panic
  site 37:21 1 an array is created here
  site 38:5 2 a place computed by a `ref mut` function is not proved to exist
  site 39:5 2 a place computed by a `ref mut` function is not proved to exist
  site 39:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 39:5 2 a place computed by a `ref mut` function is not proved to exist
  site 41:13 1 a `dyn` value is created here
  site 44:32 1 a value of a recursive enum is created here
  site 44:36 1 a value of a recursive enum is created here
  site 46:15 1 an array of the characters of a string is created here
  site 49:38 2 this index is not proved to be in range: compare it with the length first, or use `get`
  site 49:17 1 a string is built here
  site 51:19 1 a string is built here
  site 51:5 2 the program can stop here: this `panic` or `assert` is not proved to be out of reach
  site 52:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 52:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  call 38:5 10
  call 39:5 10
  call 52:5 5
fn 9 Sq.twice: none
  call 6:29 0
fn 10 grow (place): alloc
  site 14:13 1 `push` can make the array grow
  site 15:5 1 an array is created here
  unproved
