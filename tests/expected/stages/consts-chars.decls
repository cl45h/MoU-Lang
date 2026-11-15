module 0 -
module 1 std.text
global 0 ABC priv const 0
global 0 BACK priv const 8
global 0 CUT priv const 6
global 0 HALF priv const 3
global 0 NEG priv const 7
global 0 PI priv const 1
global 0 SUM priv const 5
global 0 TINY priv const 2
global 0 Z priv const 4
global 0 letters priv fn 32
global 0 main priv fn 33
global 0 text priv module 1
global 1 Builder pub struct 0
global 1 bytes pub fn 30
global 1 chars pub fn 0
global 1 concat pub fn 3
global 1 contains pub fn 26
global 1 count pub fn 1
global 1 digit pub fn 17
global 1 ends_with pub fn 24
global 1 find pub fn 25
global 1 from_bytes pub fn 31
global 1 from_chars pub fn 2
global 1 is_alpha pub fn 7
global 1 is_blank pub fn 10
global 1 is_digit pub fn 6
global 1 is_letter pub fn 8
global 1 is_lower pub fn 12
global 1 is_number pub fn 9
global 1 is_space pub fn 5
global 1 is_upper pub fn 11
global 1 join pub fn 21
global 1 lines pub fn 20
global 1 lower pub fn 16
global 1 parse_float pub fn 29
global 1 parse_int pub fn 28
global 1 raw priv import
global 1 repeat pub fn 27
global 1 split pub fn 18
global 1 starts_with pub fn 23
global 1 to_lower pub fn 14
global 1 to_upper pub fn 13
global 1 trim pub fn 22
global 1 upper pub fn 15
global 1 width priv fn 4
global 1 words pub fn 19
import 1 raw std.raw
struct 0 Builder 1 false []
  field parts [str] false
fn 0 0 false [] (let s: str false) -> [char]
fn 1 1 false [] (let s: str false) -> i64
fn 2 2 false [] (let cs: [char] false) -> str
fn 3 3 false [] (let parts: [str] false) -> str
fn 4 6 false [] (let c: char false) -> i64
fn 5 7 false [] (let c: char false) -> bool
fn 6 8 false [] (let c: char false) -> bool
fn 7 9 false [] (let c: char false) -> bool
fn 8 10 false [] (let c: char false) -> bool
fn 9 11 false [] (let c: char false) -> bool
fn 10 12 false [] (let c: char false) -> bool
fn 11 13 false [] (let c: char false) -> bool
fn 12 14 false [] (let c: char false) -> bool
fn 13 15 false [] (let c: char false) -> char
fn 14 16 false [] (let c: char false) -> char
fn 15 17 false [] (let s: str false) -> str
fn 16 18 false [] (let s: str false) -> str
fn 17 19 false [] (let c: char false) -> i64?
fn 18 20 false [] (let s: str false, let sep: char false) -> [str]
fn 19 21 false [] (let s: str false) -> [str]
fn 20 22 false [] (let s: str false) -> [str]
fn 21 23 false [] (let parts: [str] false, let sep: str false) -> str
fn 22 24 false [] (let s: str false) -> str
fn 23 25 false [] (let s: str false, let prefix: str false) -> bool
fn 24 26 false [] (let s: str false, let suffix: str false) -> bool
fn 25 27 false [] (let s: str false, let part: str false) -> i64?
fn 26 28 false [] (let s: str false, let part: str false) -> bool
fn 27 29 false [] (let s: str false, let times: i64 false) -> str
fn 28 30 false [] (let s: str false) -> i64?
fn 29 31 false [] (let s: str false) -> f64?
fn 30 32 false [] (let s: str false) -> [u8]
fn 31 33 false [] (let data: [u8] false) -> str?
fn 32 34 false [] () -> str
fn 33 44 false [] () -> i64
method std.text.Builder add mut [] (let piece: str false) -> () at=5.1 self=S0[] default=_ env= of=_
method std.text.Builder add_char mut [] (let c: char false) -> () at=5.2 self=S0[] default=_ env= of=_
method std.text.Builder build let [] () -> str at=5.5 self=S0[] default=_ env= of=_
method std.text.Builder is_empty let [] () -> bool at=5.4 self=S0[] default=_ env= of=_
method std.text.Builder line mut [] (let piece: str false) -> () at=5.3 self=S0[] default=_ env= of=_
method std.text.Builder new _ [] () -> S0[] at=5.0 self=S0[] default=_ env= of=_
impl 5 std.text.Builder S0[]
suffix 5 -
skipped_exprs 0
