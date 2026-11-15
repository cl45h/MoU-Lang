module 0 -
global 0 Shape priv trait 0
global 0 Sq priv struct 0
global 0 Wide priv struct 1
global 0 add priv fn 7
global 0 apply priv fn 0
global 0 both priv fn 1
global 0 double priv fn 5
global 0 io priv import
global 0 main priv fn 9
global 0 measure priv fn 2
global 0 run priv fn 4
global 0 shout priv fn 8
global 0 square priv fn 6
global 0 total priv fn 3
import 0 io std.io
struct 0 Sq 0 false []
  field s i64 false
struct 1 Wide 0 false []
  field w i64 false
  field h i64 false
fn 0 5 false [] (let f: fn(let i64) -> i64 false, let x: i64 false) -> i64
fn 1 6 false [] (let f: fn(let i64, let i64) -> i64 false, let x: i64 false) -> i64
fn 2 7 false [] (let s: dyn0[] false) -> i64
fn 3 8 false [] (let xs: [i64] false) -> i64
fn 4 9 false [] (let f: fn() -> i64 false) -> i64
fn 5 10 false [] (let n: i64 false) -> i64
fn 6 11 false [] (let n: i64 false) -> i64
fn 7 12 false [] (let a: i64 false, let b: i64 false) -> i64
fn 8 13 false [] () -> i64
fn 9 14 false [] () -> () ! io.Error
trait 0 Shape [] 
  method area let () -> i64
method Sq area let [] () -> i64 at=3.0 self=S0[] default=_ env= of=_
method Wide area let [] () -> i64 at=4.0 self=S1[] default=_ env= of=_
impl 3 Sq S0[]
impl 4 Wide S1[]
suffix 3 -
suffix 4 -
implements 0 Sq [] S0[] []
implements 0 Wide [] S1[] []
skipped_exprs 0
