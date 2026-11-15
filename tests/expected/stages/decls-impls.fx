error 4:1 what `Named` requires must be a trait
error 4:1 trait `Named` cannot require itself
error 5:5 method `name` is already declared by `Named`
error 11:1 trait `Self2` cannot require itself
error 16:1 `P` already implements `Into[str]`
error 16:24 method `into` is already defined for `P`
error 19:1 `Q` already implements `Into`: to implement it again, every `impl` must give the trait type arguments without type parameters
error 20:1 method `name` of `P` does not have the signature `Named` declares
error 20:1 method `first` of `P` does not have the signature `Named` declares
error 25:1 an `impl` for `Q[i64]` is not supported: a generic `impl` must be for a generic struct or enum and name all of its parameters, as in `impl[T] List[T]`
error 26:1 `impl Named for Opt` is missing the method `conv2`
error 26:1 `impl Named for Opt` is missing the method `first`
error 26:1 `impl Named for Opt` is missing the method `peek`
error 26:1 `Named` requires `Into[str]`, which `Opt` does not implement: add `impl Into[str] for Opt`
error 27:5 type parameter `T` is already a parameter of the `impl`
error 28:5 method `name` is already defined for `Opt`
error 30:1 method `show` of `P` does not have the signature `Shown` declares
error 31:1 `Secret` is private to the module `other`: it is not declared `pub`
error 33:1 method `show` of `[i64]` does not have the signature `Shown` declares
error 38:5 method `get` is already defined for `P`
error 39:5 unknown type `Missing`
error 40:1 unknown type `Missing`
error 42:1 an `impl` for `Opt[i64]` is not supported: a generic `impl` must be for a generic struct or enum and name all of its parameters, as in `impl[T] List[T]`
error 43:1 trait `Into` takes 1 type arguments, found 0
error 43:1 method `into` of `other.Open` does not have the signature `Into[_]` declares
