fn 0 clear: none
fn 1 grow: alloc
  site 3:34 1 `push` can make the array grow
fn 2 main: none
error 3:34 `grow` is in a `system` module, which has no allocator, but `push` can make the array grow
