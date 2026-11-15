fn 0 first: none
  unproved
fn 1 item: none
  unproved
fn 2 twice: panic
  site 6:27 2 `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first
fn 3 say: io
  site 7:23 4 `io.print` writes to the output
fn 4 case0: panic
  site 11:1 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 11:1 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
fn 5 case1: none
fn 6 case2: none
fn 7 case3: none
fn 8 case4: alloc
  site 35:14 1 an array is created here
fn 9 case5: alloc
  site 41:10 1 an array is created here
fn 10 case6: alloc
  site 47:10 1 a value that holds an array is copied here
fn 11 case7: alloc
  site 53:9 1 a value that holds an array is copied here
fn 12 case8: alloc
  site 60:10 1 `push` can make the array grow
fn 13 case9: alloc
  site 66:9 1 a string is built here
fn 14 case10: alloc panic
  site 72:10 1 a part of an array is copied here
  site 72:10 2 this range is not proved to be inside the array
fn 15 case11: alloc
  site 78:25 1 a value of a recursive enum is created here
fn 16 case12: alloc
  site 84:9 1 a closure that captures values is created here
fn 17 case13: panic
  call 90:1 19
fn 18 main: none
fn 19 closure: panic
  site 84:29 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
error 35:14 `case4` is `@noalloc`, but an array is created here
error 41:10 `case5` is `@noalloc`, but an array is created here
error 47:10 `case6` is `@noalloc`, but a value that holds an array is copied here
error 53:9 `case7` is `@noalloc`, but a value that holds an array is copied here
error 60:10 `case8` is `@noalloc`, but `push` can make the array grow
error 66:9 `case9` is `@noalloc`, but a string is built here
error 72:10 `case10` is `@noalloc`, but a part of an array is copied here
error 78:25 `case11` is `@noalloc`, but a value of a recursive enum is created here
error 84:9 `case12` is `@noalloc`, but a closure that captures values is created here
