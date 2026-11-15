error 6:15 literal `300` is out of range for `u8`
error 7:17 constant `EARLY` is used before its definition
error 11:13 operator `+` cannot be applied to `integer literal` and `str`
error 12:18 initializer: expected `str`, found `integer literal`
error 13:13 undefined name `undefined_name`
error 14:13 `Pair` is a type, not a value
error 15:13 `helper` is a module, not a value
error 16:13 operator `-` cannot be applied to `bool`
error 17:13 operator `!` cannot be applied to `integer literal`
error 18:13 operator `<` cannot be applied to `integer literal` and `str`
error 19:13 operator `&&` cannot be applied to `bool` and `integer literal`
error 20:17 array elements have different types: `integer literal` and `str`
error 21:13 `if` branches have different types: `integer literal` and `str`
error 22:17 literal `256` is out of range for `u8`
error 23:17 literal `-129` is out of range for `i8`
error 24:13 invalid integer literal `99999999999999999999999999999999999999999999`
error 25:13 invalid integer literal `0x`
error 26:13 cannot cast `integer literal` to `str`
error 27:13 cannot cast `str` to `i64`
error 30:13 type `(integer literal, integer literal)` has no field `5`
error 33:13 a `str` is UTF-8 and can only be indexed with a range
error 35:20 an index must be an integer or a range, found `bool`
error 36:13 a value of type `integer literal` cannot be indexed
error 37:5 cannot assign to `a`: it is immutable (declare it with `var` or take it as `mut`)
error 38:5 cannot assign to constant `BIG`
error 39:5 this expression cannot be assigned to
error 40:12 expected return type `i64`, found `str`
error 44:5 `break` outside of a loop
error 45:5 `continue` outside of a loop
error 46:14 cannot iterate over a value of type `integer literal`
error 47:19 a tuple pattern cannot match a value of type `i64`
error 48:20 the pattern of a `for` must always match
error 48:20 pattern `Some` cannot match a value of type `i64`
error 49:11 condition: expected `bool`, found `integer literal`
error 51:9 `break` values have different types (found `str`)
error 52:18 only `loop` can `break` with a value
error 53:13 this `match` on `usize` does not cover every value: add a `_` arm
error 54:14 `match` arms have different types: `integer literal` and `str`
error 56:5 this `match` does not cover every value: missing `Green`, `Blue`
error 58:25 `Color` has no variant `Purple`
error 59:29 `Blue` has 1 fields, the pattern has 2
error 61:5 this `match` does not cover every value: missing `false`
error 62:5 this `match` does not cover every value: missing `None`
error 63:33 the tuple has 2 elements, the pattern has 3
error 65:31 `Color` has no variant `Pair`
error 67:19 this pattern may not match: use `if let` or `match`
error 68:18 a tuple pattern cannot match a value of type `i64`
error 69:26 `Pair` has no field `zz`
error 70:29 pattern does not mention every field of `Pair`: add `..`
error 72:18 every alternative of an `|` pattern must bind the same names
error 73:9 a pattern of type `str` cannot match a value of type `integer literal`
error 77:22 a slice pattern cannot match a value of type `integer literal`
error 79:22 pattern `Some` cannot match a value of type `integer literal`
error 80:21 initializer: expected `i64`, found `None`
error 82:15 `?` on an optional needs a function that returns an optional
error 83:15 `?` on a result needs a function that declares an error (`! E`)
error 84:18 `catch` needs a result with an error, found `integer literal`
error 85:19 the `catch` handler must produce `str`, found `integer literal`
error 86:5 this result may hold an error and is ignored: use `?` or `catch`
error 91:5 expected 0 arguments, found 1
error 92:5 expected 1 arguments, found 0
error 93:5 expected 1 arguments, found 2
error 94:20 argument 1 is named `xs`, not `wrong`
error 95:13 argument: expected `[i64]`, found `str`
error 97:5 a value of type `i64` cannot be called
error 98:5 expected 2 arguments, found 1
error 99:13 argument: expected `str`, found `integer literal`
error 100:13 argument 1 is named `a`, not `b`
error 100:19 argument 2 is named `b`, not `a`
error 101:5 variant `Color.Blue` needs arguments
error 102:5 expected 0 arguments, found 1
error 103:5 `Color` has no variant `Nope`
error 104:16 argument: expected `i64`, found `str`
error 105:5 `private` is private to the module `helper`: it is not declared `pub`
error 106:5 the module `helper` has no `missing`
error 107:20 argument: expected `i64`, found `str`
error 108:5 `Hidden` is private to the module `helper`: it is not declared `pub`
error 110:5 expected 0 arguments, found 1
error 111:5 the method `secret` is private to the module `helper`: it is not declared `pub`
error 112:5 type `helper.Point` has no method `nothing`
error 113:5 expected 0 arguments, found 1
error 114:5 `sum` takes `self`: call it on a value of type `helper.Point`
error 115:5 `origin` has no `self`: call it as `helper.Point.origin(...)`
error 116:15 this parameter is `mut`: write `mut p` at the call
error 117:19 cannot use `r` as `mut`: it is not mutable
error 118:19 a `mut` access needs a variable, field or element
error 119:19 argument is marked `mut` but the parameter is not `mut`
error 120:19 argument is marked `own` but the parameter only borrows
error 121:19 cannot move `r`: it is borrowed (take the parameter as `own`)
error 122:19 `own` moves a whole variable: this expression is not one
error 122:19 argument: expected `Pair`, found `i64`
error 123:26 `p` is accessed twice by this call and one access is `mut`: exclusivity requires they do not overlap
error 123:26 argument is marked `mut` but the parameter is not `mut`
error 124:22 `p` is accessed twice by this call and one access is `mut`: exclusivity requires they do not overlap
error 125:5 cannot assign to `r`: it is immutable (declare it with `var` or take it as `mut`)
error 126:11 assignment: expected `i64`, found `str`
error 127:5 operator `+` cannot be applied to `str` and `integer literal`
error 129:5 `Some` takes 1 argument, found 2
error 130:5 `Some` takes 1 argument, found 0
error 131:11 argument: expected `str`, found `integer literal`
error 132:12 argument: expected `bool`, found `integer literal`
error 133:18 argument: expected `str`, found `integer literal`
error 134:5 `unsafe` is only allowed in a module that starts with `@profile(system)`
error 135:5 the contexts there are for now are `with alloc = arena { }`, `with io = lines { }`, `with err = lines { }`, `with input = lines { }`, `with args = list { }`, `with env = pairs { }` and `with files = pairs { }`
error 145:5 a fixed-size array cannot change its length: give the variable the type `[i64]`
error 147:5 the element type of this array is not known: give the variable a type, as in `var xs: [int] = []`
error 149:13 argument: expected `i64`, found `str`
error 150:5 expected 0 arguments, found 1
error 151:20 argument: expected `usize`, found `str`
error 153:19 cannot modify `zs` while a `for` iterates over it
error 154:34 cannot use `zs` as `mut`: a closure captures it by value
error 156:15 the type of the parameter `a` is not known here: write it, as in `fn(a: int)`
error 159:12 expected return type `()`, found `integer literal`
error 163:13 `?` would return an error of type `io.Error`, but the function declares `str`
error 165:12 undefined name `Error`
error 169:13 `?` on a result needs a function that declares an error (`! E`)
error 170:5 this result may hold an error and is ignored: use `?` or `catch`
error 175:13 `?` on a result needs a function that declares an error (`! E`)
error 183:13 `u8.truncate` takes one whole number
error 184:25 `u8.saturate` takes a whole number, found `f64`
error 185:17 initializer: expected `u8`, found `u8?`
error 187:13 undefined name `f64`
error 199:5 to go over a `Odd` with `for`, its method `iter` has to take `self` and nothing else
error 200:14 cannot iterate over a value of type `Plain`
