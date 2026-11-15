error 2:1 `S` is a value parameter of a type, and for now it has to be an `int`
error 6:1 `N`, a parameter of `Grid`, takes a number known when compiling
error 6:1 `T`, a parameter of `Grid`, takes a type
error 6:1 `Grid` takes 2 type arguments, found 1: write `Grid[...]`
error 6:1 `Maybe` takes types, not numbers
error 9:1 `NAME` is not a number known when compiling
error 10:1 `N` is a value parameter of a type, and for now it has to be an `int`
module 0 -
global 0 Grid priv struct 0
global 0 Maybe priv enum 0
global 0 NAME priv const 1
global 0 SIDE priv const 0
global 0 Wrong priv struct 1
global 0 f priv fn 0
global 0 g priv fn 1
global 0 h priv fn 2
global 0 i priv fn 3
struct 0 Grid 0 false [T, N]
  field cells ['T; N] false
struct 1 Wrong 0 false [S]
  field x i64 false
enum 0 Maybe 0 false [T]
  variant Yes (_: 'T)
  variant No ()
fn 0 5 false [] (let a: _ false, let b: _ false, let c: _ false, let d: S0[i64, =2] false, let e: _ false) -> i64
fn 1 6 false [] (let x: i64 false) -> S0[i64, =2]
fn 2 7 false [] (let x: i64 false) -> ()
fn 3 8 false [] (let a: S0[i64, =2] false, let b: S0[i64, _] false) -> i64
method Grid size let [T: , N: ] () -> i64 at=9.0 self=S0['T, 'N] default=_ env= of=_
impl 9 Grid S0['T, 'N]
suffix 9 -
skipped_exprs 0
