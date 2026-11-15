module 0 -
global 0 apply priv fn 3
global 0 double priv fn 0
global 0 io priv import
global 0 loud priv fn 2
global 0 main priv fn 6
global 0 pick priv fn 5
global 0 risky priv fn 1
global 0 silent priv fn 4
import 0 io std.io
fn 0 0 false [] (let x: i64 false) -> i64
fn 1 1 false [] (let x: i64 false) -> i64
fn 2 2 false [] (let x: i64 false) -> i64
fn 3 3 false [] (let f: fn(let i64) -> i64 @nopanic false, let x: i64 false) -> i64
fn 4 4 false [] (let f: fn(let i64) -> i64 @nopanic @noio false, let x: i64 false) -> i64
fn 5 5 false [] (let n: i64 false) -> fn(let i64) -> i64 @nopanic
fn 6 6 false [] () -> ()
skipped_exprs 0
