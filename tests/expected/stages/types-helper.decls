module 0 -
global 0 Hidden priv struct 1
global 0 LIMIT pub const 0
global 0 Point pub struct 0
global 0 Shape pub enum 0
global 0 private priv fn 1
global 0 visible pub fn 0
struct 0 Point 0 false []
  field x i64 false
  field y i64 false
struct 1 Hidden 0 false []
  field v i64 false
enum 0 Shape 0 false []
  variant Dot ()
  variant Circle (_: i64)
  variant Rect (w: i64, h: i64)
fn 0 2 false [] (let a: i64 false) -> i64
fn 1 3 false [] (let a: i64 false) -> i64
method Point origin _ [] () -> S0[] at=5.2 self=S0[] default=_ env= of=_
method Point secret let [] () -> i64 at=5.1 self=S0[] default=_ env= of=_
method Point sum let [] () -> i64 at=5.0 self=S0[] default=_ env= of=_
impl 5 Point S0[]
suffix 5 -
skipped_exprs 0
