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
module 0 -
module 1 other
global 0 Into priv trait 2
global 0 Named priv trait 3
global 0 Opt priv enum 1
global 0 P priv struct 3
global 0 Q priv struct 4
global 0 Self2 priv trait 5
global 0 Wide priv trait 4
global 0 main priv fn 0
global 0 other priv module 1
global 1 Box pub struct 2
global 1 Color pub enum 0
global 1 Hidden priv struct 1
global 1 Open pub struct 0
global 1 Pair pub alias
global 1 Secret priv trait 1
global 1 Shown pub trait 0
struct 0 Open 1 false []
  field x i64 false
struct 1 Hidden 1 false []
  field y i64 false
struct 2 Box 1 false [T]
  field item 'T false
struct 3 P 0 false []
  field x i64 false
struct 4 Q 0 false [T]
  field item 'T false
enum 0 Color 1 false []
  variant Red ()
  variant Green (_: i64)
  variant Blue (r: i64, g: i64)
enum 1 Opt 0 false [T]
  variant Nothing ()
  variant Just (_: 'T)
fn 0 33 false [] () -> ()
trait 0 Shown [] 
  method show let () -> str
trait 1 Secret [] 
  method s let () -> i64
trait 2 Into [T] 
  method into let () -> 'T
  method twice let () -> ('T, 'T)
  default twice 7 1
trait 3 Named [] T2[str]
  method name let () -> str
  method conv let (let x: 'T false) -> 'T
  method conv2 let (let x: 'U false) -> 'U
  method first mut () -> i64
  method peek let () -> i64
  ref first true
  ref peek false
  own conv [T: ]
  own conv2 [U: T2['U]]
trait 4 Wide [] 
trait 5 Self2 [] 
method Opt conv let [T: , T: ] (let x: 'T false) -> 'T at=21.1 self=E1['T] default=_ env= of=_
method Opt name let [T: ] () -> str at=21.2 self=E1['T] default=_ env= of=_
method P conv let [V: ] (let x: 'V false) -> 'V at=19.1 self=S3[] default=_ env= of=_
method P conv2 let [W: T2['W]] (let x: 'W false) -> 'W at=19.2 self=S3[] default=_ env= of=_
method P first mut [] () -> i64 at=19.3 self=S3[] default=_ env= of=_
method P get let [] () -> i64 at=28.1 self=S3[] default=_ env= of=_
method P into@Into[i64] let [] () -> i64 at=16.0 self=S3[] default=_ env= of=_
method P into@Into[str] let [] () -> str at=15.0 self=S3[] default=_ env= of=_
method P name let [] () -> i64 at=19.0 self=S3[] default=_ env= of=_
method P noself _ [] (let a: _ false) -> E0[] at=28.2 self=S3[] default=_ env= of=_
method P peek let [] () -> i64 at=19.4 self=S3[] default=_ env= of=_
method P show mut [] () -> str at=23.0 self=S3[] default=_ env= of=_
method P twice@Into[i64] let [] () -> (i64, i64) at=16.-1 self=S3[] default=7.1 env=T=i64 of=T2[i64]
method P twice@Into[str] let [] () -> (str, str) at=14.-1 self=S3[] default=7.1 env=T=str of=T2[str]
method P z let [] () -> () at=22.0 self=S3[] default=_ env= of=_
method Q get let [T: T0[]] () -> 'T at=27.0 self=S4['T] default=_ env= of=_
method Q into@Into[T] let [T: ] () -> 'T at=17.0 self=S4['T] default=_ env= of=_
method Q into@Into[i64] let [T: ] () -> i64 at=18.0 self=S4['T] default=_ env= of=_
method Q make _ [T: T0[]] (let item: 'T false) -> S4['T] at=27.1 self=S4['T] default=_ env= of=_
method Q map let [T: T0[], U: ] (let f: fn(let 'T) -> 'U false) -> S4['U] at=27.2 self=S4['T] default=_ env= of=_
method Q twice@Into[T] let [T: ] () -> ('T, 'T) at=17.-1 self=S4['T] default=7.1 env=T='T of=T2['T]
method [i64] show let [] (let extra: i64 false) -> str at=26.0 self=[i64] default=_ env= of=_
method i64 show let [] () -> str at=25.0 self=i64 default=_ env= of=_
method other.Open into@Into[_] let [] () -> i64 at=32.0 self=S0[] default=_ env= of=_
method other.Open twice@Into[_] let [] () -> (_, _) at=32.-1 self=S0[] default=7.1 env=T=_ of=T2[_]
impl 14 P S3[]
impl 15 P S3[]
impl 16 P S3[]
impl 17 Q S4['T]
impl 18 Q S4['T]
impl 19 P S3[]
impl 21 Opt E1['T]
impl 22 P S3[]
impl 23 P S3[]
impl 24 P S3[]
impl 25 i64 i64
impl 26 [i64] [i64]
impl 27 Q S4['T]
impl 28 P S3[]
impl 32 other.Open S0[]
suffix 14 @Into[str]
suffix 15 @Into[str]
suffix 16 @Into[i64]
suffix 17 @Into[T]
suffix 18 @Into[i64]
suffix 19 -
suffix 21 -
suffix 22 -
suffix 23 -
suffix 24 -
suffix 25 -
suffix 26 -
suffix 27 -
suffix 28 -
suffix 32 @Into[_]
implements 0 P [] S3[] []
implements 0 [i64] [] [i64] []
implements 0 i64 [] i64 []
implements 2 P [str] S3[] []
implements 2 P [i64] S3[] []
implements 2 Q ['T] S4['T] [T: ]
implements 2 other.Open [_] S0[] []
implements 3 Opt [] E1['T] [T: ]
implements 3 P [] S3[] []
overload P into into@Into[str] T2[str]
overload P into into@Into[i64] T2[i64]
overload P twice twice@Into[str] T2[str]
overload P twice twice@Into[i64] T2[i64]
overload Q into into@Into[T] T2['T]
overload Q into into@Into[i64] T2[i64]
overload Q twice twice@Into[T] T2['T]
overload other.Open into into@Into[_] T2[_]
overload other.Open twice twice@Into[_] T2[_]
alias 1 Pair todo 5
skipped trait Wide (region and value parameters)
skipped conformance of `P` to a trait that is not checked
skipped conformance of `P` to a trait that is not checked
skipped generic impl
skipped_exprs 0
