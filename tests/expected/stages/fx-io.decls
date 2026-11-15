module 0 -
global 0 P priv struct 0
global 0 Tree priv enum 0
global 0 case0 priv fn 4
global 0 case1 priv fn 5
global 0 case2 priv fn 6
global 0 first priv fn 0
global 0 io priv import
global 0 item priv fn 1
global 0 main priv fn 7
global 0 say priv fn 3
global 0 twice priv fn 2
import 0 io std.io
struct 0 P 0 false []
  field x i64 false
  field tags [str] false
enum 0 Tree 0 false []
  variant Leaf ()
  variant Node (_: E0[]?, _: i64)
fn 0 2 false [] (let xs: [i64] false) -> i64
fn 1 3 false [] (let xs: [i64] false, let i: usize false) -> i64
fn 2 4 false [] (let n: i64 false) -> i64
fn 3 5 false [] () -> () ! io.Error
fn 4 6 false [] (let xs: [i64] false, let i: usize false, let n: i64 false, let p: S0[] false, let g: fn(let i64) -> i64 false) -> i64
fn 5 7 false [] (let xs: [i64] false, let i: usize false, let n: i64 false, let p: S0[] false, let g: fn(let i64) -> i64 false) -> i64
fn 6 8 false [] (let xs: [i64] false, let i: usize false, let n: i64 false, let p: S0[] false, let g: fn(let i64) -> i64 false) -> i64
fn 7 9 false [] () -> ()
skipped_exprs 0
