error 6:1 `Top` requires `Base`, which `S` does not implement: add `impl Base for S`
error 7:1 `Mid` requires `Base`, which `S` does not implement: add `impl Base for S`
error 13:1 trait `Top` takes 1 type arguments, found 0
module 0 -
global 0 Base priv trait 0
global 0 Mid priv trait 1
global 0 S priv struct 0
global 0 Top priv trait 2
global 0 W priv struct 1
global 0 main priv fn 1
global 0 use_it priv fn 0
struct 0 S 0 false []
  field x i64 false
struct 1 W 0 false [T]
  field x 'T false
fn 0 12 false [T: T2[i64] + T0[]] (let x: 'T false, let d: dyn2[i64] false, let e: dyn2[_] false) -> i64
fn 1 13 false [] () -> ()
trait 0 Base [] 
  method base let () -> i64
trait 1 Mid [] T0[]
  method mid let () -> i64
  default mid 1 0
trait 2 Top [T] T1[]
  method top let () -> 'T
method S mid let [] () -> i64 at=6.-1 self=S0[] default=1.0 env= of=T1[]
method S top@Top[str] let [] () -> str at=5.0 self=S0[] default=_ env= of=_
method W base let [T: ] () -> i64 at=9.0 self=S1['T] default=_ env= of=_
method W mid let [T: ] () -> i64 at=8.0 self=S1['T] default=_ env= of=_
method W top@Top[T] let [T: ] () -> 'T at=7.0 self=S1['T] default=_ env= of=_
method str base let [] () -> i64 at=10.0 self=str default=_ env= of=_
method str mid let [] () -> i64 at=11.-1 self=str default=1.0 env= of=T1[]
impl 5 S S0[]
impl 6 S S0[]
impl 7 W S1['T]
impl 8 W S1['T]
impl 9 W S1['T]
impl 10 str str
impl 11 str str
suffix 5 @Top[str]
suffix 6 -
suffix 7 @Top[T]
suffix 8 -
suffix 9 -
suffix 10 -
suffix 11 -
implements 0 W [] S1['T] [T: ]
implements 0 str [] str []
implements 1 S [] S0[] []
implements 1 W [] S1['T] [T: ]
implements 1 str [] str []
implements 2 S [str] S0[] []
implements 2 W ['T] S1['T] [T: ]
overload S top top@Top[str] T2[str]
overload W top top@Top[T] T2['T]
skipped_exprs 0
