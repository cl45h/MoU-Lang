module 0 -
module 1 cabs
global 0 cabs priv module 1
global 0 calls priv fn 5
global 0 fib priv fn 1
global 0 foreign priv fn 7
global 0 io priv import
global 0 main priv fn 8
global 0 pure priv fn 6
global 0 reads priv fn 4
global 0 starts priv fn 2
global 0 waits priv fn 3
global 1 lib priv import
global 1 magnitude pub fn 0
import 0 io std.io
import 1 lib stdlib.h
externid 1 lib abs 0
extern abs stdlib.h (let n: i32 false) -> i32
fn 0 0 false [] (let n: i32 false) -> i32
fn 1 1 false [] (let n: i64 false) -> i64
fn 2 2 false [] () -> task i64
fn 3 3 false [] (let t: task i64 false) -> i64
fn 4 4 false [] () -> str
fn 5 5 false [] (let f: fn(let i64) -> i64 false) -> i64
fn 6 6 false [] (let n: i64 false) -> i64
fn 7 7 false [] (let n: i32 false) -> i32
fn 8 8 false [] () -> () ! io.Error
skipped_exprs 0
