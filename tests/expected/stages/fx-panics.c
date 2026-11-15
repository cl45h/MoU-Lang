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
