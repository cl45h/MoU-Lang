fn 0 first: none
  unproved
fn 1 item: none
  unproved
fn 2 twice: panic
  site 6:27 2 `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first
fn 3 say: io
  site 7:23 4 `io.print` writes to the output
fn 4 case0: none
fn 5 case1: none
fn 6 case2: none
fn 7 case3: none
fn 8 case4: none
fn 9 case5: none
fn 10 case6: none
fn 11 case7: panic
  site 46:1 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
fn 12 case8: panic
  site 51:1 2 `-` on `i64` can overflow here: use `-%` or `-|`, or check the range first
fn 13 case9: panic
  site 56:1 2 this `/` can divide by zero: check the divisor first
fn 14 case10: panic
  site 61:1 2 this `%` can divide by zero: check the divisor first
fn 15 case11: panic
  site 66:2 2 the value of this `as` is not proved to fit in `u8`
fn 16 case12: panic
  site 71:1 2 the shift of this `<<` is not proved to be in range
  site 71:1 2 this number is not proved to fit in `i64`
fn 17 case13: none
fn 18 case14: none
fn 19 case15: none
fn 20 case16: none
fn 21 case17: none
fn 22 case18: none
fn 23 case19: none
fn 24 case20: none
fn 25 case21: alloc
  site 131:17 1 an array is created here
fn 26 case22: alloc
  site 137:23 1 an array is created here
  site 137:16 1 an array is converted here, which makes a new one
fn 27 case23: none
fn 28 case24: none
fn 29 case25: panic
  site 156:1 2 this index is not proved to be in range: compare it with the length first, or use `get`
fn 30 case26: panic
  site 161:1 2 this index is not proved to be in range: compare it with the length first, or use `get`
fn 31 case27: panic
  site 166:18 2 this index is not proved to be in range: compare it with the length first, or use `get`
fn 32 case28: panic
  site 171:13 2 this index is not proved to be in range: compare it with the length first, or use `get`
fn 33 case29: alloc panic
  site 179:9 2 this index is not proved to be in range: compare it with the length first, or use `get`
  site 176:10 1 a value that holds an array is copied here
fn 34 case30: alloc
  site 186:10 1 a value that holds an array is copied here
fn 35 case31: none
fn 36 case32: panic
  site 202:1 2 the program can stop here: this `panic` or `assert` is not proved to be out of reach
fn 37 case33: panic
  site 208:12 2 the program can stop here: this `panic` or `assert` is not proved to be out of reach
fn 38 case34: none
  call 214:17 0
fn 39 case35: none
  call 219:17 1
  call 219:32 0
fn 40 case36: panic
  site 224:1 2 a `where` condition of `first` is not proved to hold here
  call 224:1 0
fn 41 case37: panic
  site 229:1 2 a `where` condition of `item` is not proved to hold here
  call 229:1 1
fn 42 case38: panic
  call 234:1 2
fn 43 case39: none
fn 44 case40: panic
  site 244:24 2 this call is recursive, and the nesting of calls has a limit
  call 244:24 44
fn 45 main: none
error 46:1 `case7` is `@nopanic`, but `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
error 51:1 `case8` is `@nopanic`, but `-` on `i64` can overflow here: use `-%` or `-|`, or check the range first
error 56:1 `case9` is `@nopanic`, but this `/` can divide by zero: check the divisor first
error 61:1 `case10` is `@nopanic`, but this `%` can divide by zero: check the divisor first
error 66:2 `case11` is `@nopanic`, but the value of this `as` is not proved to fit in `u8`
error 71:1 `case12` is `@nopanic`, but the shift of this `<<` is not proved to be in range
error 71:1 `case12` is `@nopanic`, but this number is not proved to fit in `i64`
error 156:1 `case25` is `@nopanic`, but this index is not proved to be in range: compare it with the length first, or use `get`
error 161:1 `case26` is `@nopanic`, but this index is not proved to be in range: compare it with the length first, or use `get`
error 166:18 `case27` is `@nopanic`, but this index is not proved to be in range: compare it with the length first, or use `get`
error 171:13 `case28` is `@nopanic`, but this index is not proved to be in range: compare it with the length first, or use `get`
error 179:9 `case29` is `@nopanic`, but this index is not proved to be in range: compare it with the length first, or use `get`
error 202:1 `case32` is `@nopanic`, but the program can stop here: this `panic` or `assert` is not proved to be out of reach
error 208:12 `case33` is `@nopanic`, but the program can stop here: this `panic` or `assert` is not proved to be out of reach
error 224:1 `case36` is `@nopanic`, but a `where` condition of `first` is not proved to hold here
error 229:1 `case37` is `@nopanic`, but a `where` condition of `item` is not proved to hold here
error 234:1 `case38` is `@nopanic`, but it calls `twice`, which can panic: `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first (line 6 of its file)
error 244:24 `case40` is `@nopanic`, but this call is recursive, and the nesting of calls has a limit
