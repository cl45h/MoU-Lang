module 0 -
module 1 helper
global 0 BIG priv const 1
global 0 Color priv enum 1
global 0 EARLY priv const 3
global 0 LATE priv const 2
global 0 Odd priv struct 3
global 0 Pair priv struct 2
global 0 Plain priv struct 4
global 0 basics priv fn 2
global 0 calls priv fn 4
global 0 control priv fn 3
global 0 conversions priv fn 14
global 0 fails priv fn 10
global 0 helper priv module 1
global 0 io priv import
global 0 loops priv fn 15
global 0 main priv fn 13
global 0 no_fail priv fn 11
global 0 opt priv fn 12
global 0 takes_let priv fn 6
global 0 takes_mut priv fn 5
global 0 takes_own priv fn 7
global 0 takes_two priv fn 8
global 0 xs_push priv fn 9
global 1 Hidden priv struct 1
global 1 LIMIT pub const 0
global 1 Point pub struct 0
global 1 Shape pub enum 0
global 1 private priv fn 1
global 1 visible pub fn 0
import 0 io std.io
struct 0 Point 1 false []
  field x i64 false
  field y i64 false
struct 1 Hidden 1 false []
  field v i64 false
struct 2 Pair 0 false []
  field a i64 false
  field b str false
struct 3 Odd 0 false []
  field n i64 false
struct 4 Plain 0 false []
  field n i64 false
enum 0 Shape 1 false []
  variant Dot ()
  variant Circle (_: i64)
  variant Rect (w: i64, h: i64)
enum 1 Color 0 false []
  variant Red ()
  variant Green ()
  variant Blue (_: i64)
fn 0 2 false [] (let a: i64 false) -> i64
fn 1 3 false [] (let a: i64 false) -> i64
fn 2 12 false [] () -> i64
fn 3 13 false [] (let xs: [i64] false) -> i64
fn 4 14 false [] (mut p: S2[] false, own o: S2[] false, let r: S2[] false) -> i64
fn 5 15 false [] (mut p: S2[] false) -> ()
fn 6 16 false [] (let p: S2[] false) -> ()
fn 7 17 false [] (own p: S2[] false) -> ()
fn 8 18 false [] (mut a: S2[] false, let b: S2[] false) -> ()
fn 9 19 false [] () -> ()
fn 10 20 false [] () -> i64 ! str
fn 11 21 false [] () -> i64
fn 12 22 false [] () -> i64?
fn 13 23 false [] () -> ()
fn 14 24 false [] (let n: i64 false, let x: f64 false) -> ()
fn 15 28 false [] (let o: S3[] false, let p: S4[] false) -> ()
method Odd iter mut [] () -> [i64] at=26.0 self=S3[] default=_ env= of=_
method helper.Point origin _ [] () -> S0[] at=5.2 self=S0[] default=_ env= of=_
method helper.Point secret let [] () -> i64 at=5.1 self=S0[] default=_ env= of=_
method helper.Point sum let [] () -> i64 at=5.0 self=S0[] default=_ env= of=_
impl 5 helper.Point S0[]
impl 26 Odd S3[]
suffix 5 -
suffix 26 -
skipped_exprs 0
