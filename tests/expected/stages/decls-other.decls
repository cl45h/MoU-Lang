module 0 -
global 0 Box pub struct 2
global 0 Color pub enum 0
global 0 Hidden priv struct 1
global 0 Open pub struct 0
global 0 Pair pub alias
global 0 Secret priv trait 1
global 0 Shown pub trait 0
struct 0 Open 0 false []
  field x i64 false
struct 1 Hidden 0 false []
  field y i64 false
struct 2 Box 0 false [T]
  field item 'T false
enum 0 Color 0 false []
  variant Red ()
  variant Green (_: i64)
  variant Blue (r: i64, g: i64)
trait 0 Shown [] 
  method show let () -> str
trait 1 Secret [] 
  method s let () -> i64
alias 0 Pair todo 5
skipped_exprs 0
