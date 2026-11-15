module 0 -
global 0 Area priv trait 0
global 0 BIG priv const 1
global 0 Color priv enum 1
global 0 NAMES priv const 2
global 0 Node priv struct 1
global 0 P priv struct 0
global 0 PI priv const 0
global 0 Sq priv struct 2
global 0 Tree priv enum 0
global 0 grow priv fn 0
global 0 main priv fn 4
global 0 note priv fn 3
global 0 shapes priv fn 2
global 0 sum priv fn 1
struct 0 P 0 false []
  field x i64 false
  field y f64 false
  field ok bool false
  field c char false
struct 1 Node 0 false []
  field next S1[]? false
  field v i64 false
struct 2 Sq 0 false []
  field s f64 false
  field tags [str] true
  field n i64 true
enum 0 Tree 0 false []
  variant Leaf ()
  variant Fork (_: E0[], _: E0[])
  variant Tip (_: i64)
enum 1 Color 0 false []
  variant Red ()
  variant Green ()
fn 0 10 false [] (mut xs: [i64] false, let n: i64 true) -> i64
fn 1 11 false [] (let t: E0[] false) -> i64
fn 2 12 false [] (let all: [dyn0[]] false) -> f64
fn 3 13 false [] (let x: f64 false) -> ()
fn 4 14 false [] () -> i64
trait 0 Area [] 
  method area let () -> f64
  method twice let () -> f64
  default twice 4 1
method Sq area let [] () -> f64 at=6.0 self=S2[] default=_ env= of=_
method Sq twice let [] () -> f64 at=6.-1 self=S2[] default=4.1 env= of=T0[]
impl 6 Sq S2[]
suffix 6 -
implements 0 Sq [] S2[] []
skipped_exprs 0
