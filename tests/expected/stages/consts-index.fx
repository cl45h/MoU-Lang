fn 0 constant XS: alloc
  site 1:24 1 an array is created here
  site 1:17 1 an array is converted here, which makes a new one
fn 1 at: panic
  site 2:24 2 this index is not proved to be in range: compare it with the length first, or use `get`
fn 2 constant A: panic
  call 3:14 1
fn 3 main: none
