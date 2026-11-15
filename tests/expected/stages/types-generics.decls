module 0 -
module 1 std.list
module 2 std.cmp
global 0 Box priv struct 1
global 0 Conv priv trait 3
global 0 Grow priv trait 4
global 0 Make priv trait 2
global 0 Num priv struct 2
global 0 Peek priv trait 5
global 0 Show priv trait 1
global 0 Tree priv enum 1
global 0 bad_assoc priv fn 7
global 0 bad_bound priv fn 6
global 0 by_ref priv fn 13
global 0 by_ref_bad priv fn 14
global 0 by_ref_none priv fn 15
global 0 by_ref_other priv fn 16
global 0 by_ref_ret priv fn 17
global 0 cmp priv module 2
global 0 conv_it priv fn 8
global 0 first_of priv fn 10
global 0 grow_it priv fn 11
global 0 grow_ok priv fn 12
global 0 id priv fn 3
global 0 list priv module 1
global 0 main priv fn 19
global 0 make_it priv fn 5
global 0 pair priv fn 9
global 0 show_it priv fn 4
global 0 uses priv fn 18
global 1 List pub struct 0
global 1 cmp priv module 2
global 2 Ord pub trait 0
global 2 Order pub enum 0
global 2 max pub fn 1
global 2 min pub fn 0
global 2 sort pub fn 2
struct 0 List 1 false [T]
  field items ['T] false
struct 1 Box 0 false [T]
  field item 'T false
struct 2 Num 0 false []
  field n i64 false
enum 0 Order 2 false []
  variant Less ()
  variant Equal ()
  variant Greater ()
enum 1 Tree 0 false [T]
  variant Leaf ()
  variant Node (_: 'T)
fn 0 19 false [T: T0[]] (let a: 'T false, let b: 'T false) -> 'T
fn 1 20 false [T: T0[]] (let a: 'T false, let b: 'T false) -> 'T
fn 2 21 false [T: T0[]] (mut xs: ['T] false) -> ()
fn 3 40 false [T: ] (let x: 'T false) -> 'T
fn 4 41 false [T: T1[]] (let x: 'T false) -> str
fn 5 42 false [T: T2[]] () -> 'T
fn 6 43 false [T: ] (let x: 'T false) -> str
fn 7 44 false [T: T1[]] () -> 'T
fn 8 45 false [T: T3['U], U: ] (let x: 'T false) -> 'U
fn 9 46 false [A: , B: ] (let a: 'A false, let b: 'B false) -> ('A, 'B)
fn 10 47 false [T: ] (let xs: ['T] false) -> 'T?
fn 11 48 false [T: T4[]] (let x: 'T false) -> ()
fn 12 49 false [T: T4[]] (mut x: 'T false) -> ()
fn 13 50 false [] (mut n: S2[] false) -> i64
fn 14 51 false [] (let n: S2[] false) -> i64
fn 15 52 false [] () -> i64
fn 16 53 false [] (mut n: S2[] false, mut m: S2[] false) -> i64
fn 17 54 false [] (mut n: S2[] false) -> i64
fn 18 55 false [] () -> i64
fn 19 56 false [] () -> ()
trait 0 Ord [] 
  method cmp let (let other: 'Self false) -> E0[]
trait 1 Show [] 
  method show let () -> str
trait 2 Make [] 
  method make _ () -> 'Self
  method twice let () -> 'Self
  default twice 26 1
trait 3 Conv [T] 
  method conv let () -> 'T
trait 4 Grow [] 
  method grow mut () -> ()
trait 5 Peek [] 
  method first mut () -> i64
  ref first true
method Box get let [T: ] () -> 'T at=39.0 self=S1['T] default=_ env= of=_
method Box map let [T: , U: ] (let f: fn(let 'T) -> 'U false) -> S1['U] at=39.2 self=S1['T] default=_ env= of=_
method Box new _ [T: ] (let item: 'T false) -> S1['T] at=39.3 self=S1['T] default=_ env= of=_
method Box set mut [T: ] (let item: 'T false) -> () at=39.1 self=S1['T] default=_ env= of=_
method Num conv@Conv[i64] let [] () -> i64 at=36.0 self=S2[] default=_ env= of=_
method Num conv@Conv[str] let [] () -> str at=35.0 self=S2[] default=_ env= of=_
method Num first mut [] () -> i64 at=38.0 self=S2[] default=_ env= of=_
method Num grow mut [] () -> () at=37.0 self=S2[] default=_ env= of=_
method Num make _ [] () -> S2[] at=34.0 self=S2[] default=_ env= of=_
method Num show let [] () -> str at=33.0 self=S2[] default=_ env= of=_
method Num twice let [] () -> S2[] at=34.-1 self=S2[] default=26.1 env= of=T2[]
method bool cmp let [] (let other: bool false) -> E0[] at=18.0 self=bool default=_ env= of=_
method char cmp let [] (let other: char false) -> E0[] at=5.0 self=char default=_ env= of=_
method f32 cmp let [] (let other: f32 false) -> E0[] at=17.0 self=f32 default=_ env= of=_
method f64 cmp let [] (let other: f64 false) -> E0[] at=3.0 self=f64 default=_ env= of=_
method i128 cmp let [] (let other: i128 false) -> E0[] at=9.0 self=i128 default=_ env= of=_
method i16 cmp let [] (let other: i16 false) -> E0[] at=7.0 self=i16 default=_ env= of=_
method i32 cmp let [] (let other: i32 false) -> E0[] at=8.0 self=i32 default=_ env= of=_
method i64 cmp let [] (let other: i64 false) -> E0[] at=2.0 self=i64 default=_ env= of=_
method i8 cmp let [] (let other: i8 false) -> E0[] at=6.0 self=i8 default=_ env= of=_
method isize cmp let [] (let other: isize false) -> E0[] at=10.0 self=isize default=_ env= of=_
method std.list.List all let [T: ] () -> ['T] at=23.5 self=S0['T] default=_ env= of=_
method std.list.List any let [T: ] (let test: fn(let 'T) -> bool false) -> bool at=23.21 self=S0['T] default=_ env= of=_
method std.list.List at let [T: ] (let i: i64 false) -> 'T at=23.9 self=S0['T] default=_ env= of=_
method std.list.List clear mut [T: ] () -> () at=23.15 self=S0['T] default=_ env= of=_
method std.list.List contains let [T: T0[]] (let item: 'T false) -> bool at=24.2 self=S0['T] default=_ env= of=_
method std.list.List every let [T: ] (let test: fn(let 'T) -> bool false) -> bool at=23.22 self=S0['T] default=_ env= of=_
method std.list.List extend mut [T: ] (let more: ['T] false) -> () at=23.16 self=S0['T] default=_ env= of=_
method std.list.List filter let [T: ] (let keep: fn(let 'T) -> bool false) -> S0['T] at=23.19 self=S0['T] default=_ env= of=_
method std.list.List first let [T: ] () -> 'T? at=23.11 self=S0['T] default=_ env= of=_
method std.list.List fold let [T: , A: ] (let start: 'A false, let f: fn(let 'A, let 'T) -> 'A false) -> 'A at=23.20 self=S0['T] default=_ env= of=_
method std.list.List get let [T: ] (let i: i64 false) -> 'T? at=23.8 self=S0['T] default=_ env= of=_
method std.list.List index_of let [T: T0[]] (let item: 'T false) -> i64? at=24.1 self=S0['T] default=_ env= of=_
method std.list.List insert mut [T: ] (let i: i64 false, let item: 'T false) -> () at=23.13 self=S0['T] default=_ env= of=_
method std.list.List is_empty let [T: ] () -> bool at=23.4 self=S0['T] default=_ env= of=_
method std.list.List iter let [T: ] () -> ['T] at=23.3 self=S0['T] default=_ env= of=_
method std.list.List largest let [T: T0[]] () -> 'T? at=24.4 self=S0['T] default=_ env= of=_
method std.list.List last let [T: ] () -> 'T? at=23.12 self=S0['T] default=_ env= of=_
method std.list.List len let [T: ] () -> i64 at=23.2 self=S0['T] default=_ env= of=_
method std.list.List map let [T: , U: ] (let f: fn(let 'T) -> 'U false) -> S0['U] at=23.18 self=S0['T] default=_ env= of=_
method std.list.List new _ [T: ] () -> S0['T] at=23.0 self=S0['T] default=_ env= of=_
method std.list.List of _ [T: ] (let items: ['T] false) -> S0['T] at=23.1 self=S0['T] default=_ env= of=_
method std.list.List pop mut [T: ] () -> 'T? at=23.7 self=S0['T] default=_ env= of=_
method std.list.List push mut [T: ] (let item: 'T false) -> () at=23.6 self=S0['T] default=_ env= of=_
method std.list.List remove mut [T: ] (let i: i64 false) -> 'T at=23.14 self=S0['T] default=_ env= of=_
method std.list.List reverse mut [T: ] () -> () at=23.17 self=S0['T] default=_ env= of=_
method std.list.List set mut [T: ] (let i: i64 false, let item: 'T false) -> () at=23.10 self=S0['T] default=_ env= of=_
method std.list.List smallest let [T: T0[]] () -> 'T? at=24.3 self=S0['T] default=_ env= of=_
method std.list.List sort mut [T: T0[]] () -> () at=24.0 self=S0['T] default=_ env= of=_
method str cmp let [] (let other: str false) -> E0[] at=4.0 self=str default=_ env= of=_
method u128 cmp let [] (let other: u128 false) -> E0[] at=15.0 self=u128 default=_ env= of=_
method u16 cmp let [] (let other: u16 false) -> E0[] at=12.0 self=u16 default=_ env= of=_
method u32 cmp let [] (let other: u32 false) -> E0[] at=13.0 self=u32 default=_ env= of=_
method u64 cmp let [] (let other: u64 false) -> E0[] at=14.0 self=u64 default=_ env= of=_
method u8 cmp let [] (let other: u8 false) -> E0[] at=11.0 self=u8 default=_ env= of=_
method usize cmp let [] (let other: usize false) -> E0[] at=16.0 self=usize default=_ env= of=_
impl 2 i64 i64
impl 3 f64 f64
impl 4 str str
impl 5 char char
impl 6 i8 i8
impl 7 i16 i16
impl 8 i32 i32
impl 9 i128 i128
impl 10 isize isize
impl 11 u8 u8
impl 12 u16 u16
impl 13 u32 u32
impl 14 u64 u64
impl 15 u128 u128
impl 16 usize usize
impl 17 f32 f32
impl 18 bool bool
impl 23 std.list.List S0['T]
impl 24 std.list.List S0['T]
impl 33 Num S2[]
impl 34 Num S2[]
impl 35 Num S2[]
impl 36 Num S2[]
impl 37 Num S2[]
impl 38 Num S2[]
impl 39 Box S1['T]
suffix 2 -
suffix 3 -
suffix 4 -
suffix 5 -
suffix 6 -
suffix 7 -
suffix 8 -
suffix 9 -
suffix 10 -
suffix 11 -
suffix 12 -
suffix 13 -
suffix 14 -
suffix 15 -
suffix 16 -
suffix 17 -
suffix 18 -
suffix 23 -
suffix 24 -
suffix 33 -
suffix 34 -
suffix 35 @Conv[str]
suffix 36 @Conv[i64]
suffix 37 -
suffix 38 -
suffix 39 -
implements 0 bool [] bool []
implements 0 char [] char []
implements 0 f32 [] f32 []
implements 0 f64 [] f64 []
implements 0 i128 [] i128 []
implements 0 i16 [] i16 []
implements 0 i32 [] i32 []
implements 0 i64 [] i64 []
implements 0 i8 [] i8 []
implements 0 isize [] isize []
implements 0 str [] str []
implements 0 u128 [] u128 []
implements 0 u16 [] u16 []
implements 0 u32 [] u32 []
implements 0 u64 [] u64 []
implements 0 u8 [] u8 []
implements 0 usize [] usize []
implements 1 Num [] S2[] []
implements 2 Num [] S2[] []
implements 3 Num [str] S2[] []
implements 3 Num [i64] S2[] []
implements 4 Num [] S2[] []
implements 5 Num [] S2[] []
overload Num conv conv@Conv[str] T3[str]
overload Num conv conv@Conv[i64] T3[i64]
skipped_exprs 0
