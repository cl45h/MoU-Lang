error 6:5 the result of `bad`: a C function can only take and return numbers of up to 64 bits, `bool`, `str`, `ptr`, structs marked `@c` and, to take, arrays of numbers and functions of numbers
error 7:5 `worse` is a C function: it is declared with its parameters and its result, and nothing else
error 8:5 only functions can be declared for a C header
error 10:1 field `x` is defined twice in `A`
error 10:1 unknown type `Missing`
error 10:1 `Hidden` is private to the module `other`: it is not declared `pub`
error 10:1 the module `other` has no `Nope`
error 10:1 unknown type `nope.Thing`
error 11:1 `A` is already defined
error 12:1 `int` is already defined
error 13:1 variant `One` is defined twice in `E`
error 14:1 type alias `Loop` refers to itself
error 16:1 type alias `L2` refers to itself
error 19:1 `Gen` takes 1 type arguments, found 2
error 20:1 `Use` takes no type arguments
error 21:1 type alias `Rec` refers to itself
error 23:1 bounds on the type parameter `T` of a type are not supported: put them on the `impl`
error 23:1 `G` takes 1 type arguments, found 2: write `G[...]`
error 23:1 `G` takes 1 type arguments, found 0: write `G[...]`
error 25:1 `Secret` is private to the module `other`: it is not declared `pub`
error 25:1 `dyn` must be followed by a trait
error 25:1 `dyn` takes one trait: declare a trait that requires the others
error 25:1 `Self` can only be used inside an `impl`
error 25:1 `Shown` is a trait, not a type (use `dyn Shown`)
error 25:1 `Box` takes 1 type arguments, found 0: write `Box[...]`
error 26:1 unknown type `T`
error 26:1 `self` can only be used inside an `impl`
error 26:1 `Self` can only be used inside an `impl`
error 27:1 parameter `a` of `noty` needs a type
error 28:1 the bound of `T` must be a trait
error 28:1 `Secret` is private to the module `other`: it is not declared `pub`
error 28:1 the bound of `U` must be a trait
error 28:1 `int` takes no type arguments
error 29:1 `g` is already defined
module 0 -
module 1 other
module 2 std.list
module 3 std.cmp
global 0 A priv struct 4
global 0 Bad priv alias
global 0 Bad2 priv alias
global 0 E priv enum 2
global 0 G priv struct 7
global 0 Gen priv alias
global 0 L2 priv alias
global 0 L3 priv alias
global 0 Loop priv alias
global 0 R priv struct 8
global 0 Rec priv alias
global 0 Use priv alias
global 0 UseRec priv alias
global 0 UsesR priv struct 9
global 0 f priv fn 3
global 0 g priv fn 5
global 0 h priv fn 6
global 0 io priv import
global 0 list priv module 2
global 0 m priv import
global 0 main priv fn 7
global 0 noty priv fn 4
global 0 other priv module 1
global 1 Box pub struct 2
global 1 Color pub enum 0
global 1 Hidden priv struct 1
global 1 Open pub struct 0
global 1 Pair pub alias
global 1 Secret priv trait 1
global 1 Shown pub trait 0
global 2 List pub struct 3
global 2 cmp priv module 3
global 3 Ord pub trait 2
global 3 Order pub enum 1
global 3 max pub fn 1
global 3 min pub fn 0
global 3 sort pub fn 2
import 0 io std.io
import 0 m math.h
externid 0 m bad 1
externid 0 m sqrt 0
extern sqrt math.h (let x: f64 false) -> f64
extern bad math.h (let x: str false) -> i128
struct 0 Open 1 false []
  field x i64 false
struct 1 Hidden 1 false []
  field y i64 false
struct 2 Box 1 false [T]
  field item 'T false
struct 3 List 2 false [T]
  field items ['T] false
struct 4 A 0 false []
  field q i64 false
struct 5 A 0 false []
struct 6 int 0 false []
struct 7 G 0 false [T]
  field item 'T false
  field more S7['T] false
  field wrong _ false
  field none _ false
struct 8 R 0 true []
struct 9 UsesR 0 false []
  field r _ false
  field d dyn0[] false
  field d2 _ false
  field d3 _ false
  field d4 _ false
  field s _ false
  field t _ false
  field arr [i64; 4] false
  field p (i64, i64) false
  field b S2[str] false
  field bb _ false
  field c E0[] false
enum 0 Color 1 false []
  variant Red ()
  variant Green (_: i64)
  variant Blue (r: i64, g: i64)
enum 1 Order 3 false []
  variant Less ()
  variant Equal ()
  variant Greater ()
enum 2 E 0 false []
  variant One ()
  variant One ()
  variant Two (_: S4[], _: [i64]?, _: (i64, str))
  variant Three (k: fn(let i64, mut str) -> bool ! str)
fn 0 26 false [T: T2[]] (let a: 'T false, let b: 'T false) -> 'T
fn 1 27 false [T: T2[]] (let a: 'T false, let b: 'T false) -> 'T
fn 2 28 false [T: T2[]] (mut xs: ['T] false) -> ()
fn 3 48 false [] (let a: i64 false, let b: _ false) -> _
fn 4 49 false [] (let a: _ false, let b: i64 false) -> ()
fn 5 50 false [T: , U: T0[]] (let a: 'T false, let b: 'U false, let c: i64 false) -> 'T ! 'U
fn 6 52 true [] () -> ()
fn 7 53 false [] () -> ()
trait 0 Shown [] 
  method show let () -> str
trait 1 Secret [] 
  method s let () -> i64
trait 2 Ord [] 
  method cmp let (let other: 'Self false) -> E1[]
method bool cmp let [] (let other: bool false) -> E1[] at=25.0 self=bool default=_ env= of=_
method char cmp let [] (let other: char false) -> E1[] at=12.0 self=char default=_ env= of=_
method f32 cmp let [] (let other: f32 false) -> E1[] at=24.0 self=f32 default=_ env= of=_
method f64 cmp let [] (let other: f64 false) -> E1[] at=10.0 self=f64 default=_ env= of=_
method i128 cmp let [] (let other: i128 false) -> E1[] at=16.0 self=i128 default=_ env= of=_
method i16 cmp let [] (let other: i16 false) -> E1[] at=14.0 self=i16 default=_ env= of=_
method i32 cmp let [] (let other: i32 false) -> E1[] at=15.0 self=i32 default=_ env= of=_
method i64 cmp let [] (let other: i64 false) -> E1[] at=9.0 self=i64 default=_ env= of=_
method i8 cmp let [] (let other: i8 false) -> E1[] at=13.0 self=i8 default=_ env= of=_
method isize cmp let [] (let other: isize false) -> E1[] at=17.0 self=isize default=_ env= of=_
method std.list.List all let [T: ] () -> ['T] at=30.5 self=S3['T] default=_ env= of=_
method std.list.List any let [T: ] (let test: fn(let 'T) -> bool false) -> bool at=30.21 self=S3['T] default=_ env= of=_
method std.list.List at let [T: ] (let i: i64 false) -> 'T at=30.9 self=S3['T] default=_ env= of=_
method std.list.List clear mut [T: ] () -> () at=30.15 self=S3['T] default=_ env= of=_
method std.list.List contains let [T: T2[]] (let item: 'T false) -> bool at=31.2 self=S3['T] default=_ env= of=_
method std.list.List every let [T: ] (let test: fn(let 'T) -> bool false) -> bool at=30.22 self=S3['T] default=_ env= of=_
method std.list.List extend mut [T: ] (let more: ['T] false) -> () at=30.16 self=S3['T] default=_ env= of=_
method std.list.List filter let [T: ] (let keep: fn(let 'T) -> bool false) -> S3['T] at=30.19 self=S3['T] default=_ env= of=_
method std.list.List first let [T: ] () -> 'T? at=30.11 self=S3['T] default=_ env= of=_
method std.list.List fold let [T: , A: ] (let start: 'A false, let f: fn(let 'A, let 'T) -> 'A false) -> 'A at=30.20 self=S3['T] default=_ env= of=_
method std.list.List get let [T: ] (let i: i64 false) -> 'T? at=30.8 self=S3['T] default=_ env= of=_
method std.list.List index_of let [T: T2[]] (let item: 'T false) -> i64? at=31.1 self=S3['T] default=_ env= of=_
method std.list.List insert mut [T: ] (let i: i64 false, let item: 'T false) -> () at=30.13 self=S3['T] default=_ env= of=_
method std.list.List is_empty let [T: ] () -> bool at=30.4 self=S3['T] default=_ env= of=_
method std.list.List iter let [T: ] () -> ['T] at=30.3 self=S3['T] default=_ env= of=_
method std.list.List largest let [T: T2[]] () -> 'T? at=31.4 self=S3['T] default=_ env= of=_
method std.list.List last let [T: ] () -> 'T? at=30.12 self=S3['T] default=_ env= of=_
method std.list.List len let [T: ] () -> i64 at=30.2 self=S3['T] default=_ env= of=_
method std.list.List map let [T: , U: ] (let f: fn(let 'T) -> 'U false) -> S3['U] at=30.18 self=S3['T] default=_ env= of=_
method std.list.List new _ [T: ] () -> S3['T] at=30.0 self=S3['T] default=_ env= of=_
method std.list.List of _ [T: ] (let items: ['T] false) -> S3['T] at=30.1 self=S3['T] default=_ env= of=_
method std.list.List pop mut [T: ] () -> 'T? at=30.7 self=S3['T] default=_ env= of=_
method std.list.List push mut [T: ] (let item: 'T false) -> () at=30.6 self=S3['T] default=_ env= of=_
method std.list.List remove mut [T: ] (let i: i64 false) -> 'T at=30.14 self=S3['T] default=_ env= of=_
method std.list.List reverse mut [T: ] () -> () at=30.17 self=S3['T] default=_ env= of=_
method std.list.List set mut [T: ] (let i: i64 false, let item: 'T false) -> () at=30.10 self=S3['T] default=_ env= of=_
method std.list.List smallest let [T: T2[]] () -> 'T? at=31.3 self=S3['T] default=_ env= of=_
method std.list.List sort mut [T: T2[]] () -> () at=31.0 self=S3['T] default=_ env= of=_
method str cmp let [] (let other: str false) -> E1[] at=11.0 self=str default=_ env= of=_
method u128 cmp let [] (let other: u128 false) -> E1[] at=22.0 self=u128 default=_ env= of=_
method u16 cmp let [] (let other: u16 false) -> E1[] at=19.0 self=u16 default=_ env= of=_
method u32 cmp let [] (let other: u32 false) -> E1[] at=20.0 self=u32 default=_ env= of=_
method u64 cmp let [] (let other: u64 false) -> E1[] at=21.0 self=u64 default=_ env= of=_
method u8 cmp let [] (let other: u8 false) -> E1[] at=18.0 self=u8 default=_ env= of=_
method usize cmp let [] (let other: usize false) -> E1[] at=23.0 self=usize default=_ env= of=_
impl 9 i64 i64
impl 10 f64 f64
impl 11 str str
impl 12 char char
impl 13 i8 i8
impl 14 i16 i16
impl 15 i32 i32
impl 16 i128 i128
impl 17 isize isize
impl 18 u8 u8
impl 19 u16 u16
impl 20 u32 u32
impl 21 u64 u64
impl 22 u128 u128
impl 23 usize usize
impl 24 f32 f32
impl 25 bool bool
impl 30 std.list.List S3['T]
impl 31 std.list.List S3['T]
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
suffix 19 -
suffix 20 -
suffix 21 -
suffix 22 -
suffix 23 -
suffix 24 -
suffix 25 -
suffix 30 -
suffix 31 -
implements 2 bool [] bool []
implements 2 char [] char []
implements 2 f32 [] f32 []
implements 2 f64 [] f64 []
implements 2 i128 [] i128 []
implements 2 i16 [] i16 []
implements 2 i32 [] i32 []
implements 2 i64 [] i64 []
implements 2 i8 [] i8 []
implements 2 isize [] isize []
implements 2 str [] str []
implements 2 u128 [] u128 []
implements 2 u16 [] u16 []
implements 2 u32 [] u32 []
implements 2 u64 [] u64 []
implements 2 u8 [] u8 []
implements 2 usize [] usize []
alias 0 Bad done _
alias 0 Bad2 done [i64]
alias 0 Gen todo 39
alias 0 L2 done [(_, i64)]
alias 0 L3 done (_, i64)
alias 0 Loop done _
alias 0 Rec todo 43
alias 0 Use done [i64]
alias 0 UseRec done _
alias 1 Pair todo 5
skipped struct R
skipped fn h
skipped_exprs 2
