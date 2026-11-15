fn 0 chars: alloc
  site 1048584:14 1 an array of the characters of a string is created here
  site 1048584:27 1 `push` can make the array grow
fn 1 count: alloc panic
  site 1048591:14 1 an array of the characters of a string is created here
  site 1048591:18 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
fn 2 from_chars: alloc
  site 1048595:40 1 `raw.from_chars` creates its result on the heap
fn 3 concat: alloc
  site 1048598:38 1 `raw.concat` creates its result on the heap
fn 4 std.text.Builder.new: none
fn 5 std.text.Builder.add: alloc
  site 1048608:56 1 `push` can make the array grow
fn 6 std.text.Builder.add_char: alloc
  site 1048610:58 1 a string is built here
  site 1048610:58 1 `push` can make the array grow
fn 7 std.text.Builder.line: alloc
  site 1048614:25 1 `push` can make the array grow
  site 1048615:25 1 `push` can make the array grow
fn 8 std.text.Builder.is_empty: none
fn 9 std.text.Builder.build: alloc
  site 1048620:33 1 `raw.concat` creates its result on the heap
fn 10 width: panic
  site 1048625:13 2 the value of this `as` is not proved to fit in `i64`
fn 11 is_space: none
fn 12 is_digit: none
fn 13 is_alpha: none
fn 14 is_letter: none
fn 15 is_number: none
fn 16 is_blank: none
fn 17 is_upper: none
fn 18 is_lower: none
fn 19 to_upper: none
fn 20 to_lower: none
fn 21 upper: alloc
  site 1048656:14 1 an array of the characters of a string is created here
  site 1048657:5 1 `raw.from_chars` creates its result on the heap
  site 1048656:27 1 `push` can make the array grow
fn 22 lower: alloc
  site 1048663:14 1 an array of the characters of a string is created here
  site 1048664:5 1 `raw.from_chars` creates its result on the heap
  site 1048663:27 1 `push` can make the array grow
fn 23 digit: panic
  site 1048669:27 2 the value of this `as` is not proved to fit in `i64`
  site 1048669:38 2 the value of this `as` is not proved to fit in `i64`
  site 1048669:27 2 `-` on `i64` can overflow here: use `-%` or `-|`, or check the range first
  call 1048669:8 12
fn 24 split: alloc panic
  site 1048677:14 1 an array of the characters of a string is created here
  site 1048684:16 1 a part of an array is copied here
  site 1048684:16 2 this range is not proved to be inside the array
  site 1048684:16 1 `push` can make the array grow
  site 1048679:24 1 a part of an array is copied here
  site 1048679:24 2 this range is not proved to be inside the array
  site 1048679:24 1 `push` can make the array grow
  site 1048680:21 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048682:9 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  call 1048682:15 10
  call 1048680:26 10
fn 25 words: alloc panic
  site 1048694:14 1 an array of the characters of a string is created here
  site 1048696:36 1 a part of an array is copied here
  site 1048696:36 2 this range is not proved to be inside the array
  site 1048696:36 1 `push` can make the array grow
  site 1048702:9 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048704:28 1 a part of an array is copied here
  site 1048704:28 2 this range is not proved to be inside the array
  site 1048704:28 1 `push` can make the array grow
  call 1048695:12 11
  call 1048702:15 10
fn 26 lines: alloc panic
  call 1048708:33 24
fn 27 join: alloc
  site 1048716:5 1 `raw.concat` creates its result on the heap
  site 1048714:18 1 `push` can make the array grow
  site 1048713:35 1 `push` can make the array grow
fn 28 trim: alloc panic
  site 1048725:14 1 an array of the characters of a string is created here
  site 1048729:19 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048731:9 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048733:15 1 a part of an array is copied here
  site 1048733:15 2 this range is not proved to be inside the array
  call 1048726:13 11
  call 1048731:15 10
  call 1048729:24 10
fn 29 starts_with: alloc panic
  site 1048737:28 1 a part of an array is copied here
  site 1048737:28 2 this range is not proved to be inside the array
fn 30 ends_with: alloc panic
  site 1048741:30 2 `-` on `usize` can overflow here: use `-%` or `-|`, or check the range first
  site 1048741:28 1 a part of an array is copied here
  site 1048741:28 2 this range is not proved to be inside the array
fn 31 find: alloc panic
  site 1048752:11 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048754:28 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048754:24 2 this index is not proved to be in range: compare it with the length first, or use `get`
  site 1048754:38 2 this index is not proved to be in range: compare it with the length first, or use `get`
  site 1048756:21 2 this index is not proved to be in range: compare it with the length first, or use `get`
  site 1048756:9 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048757:9 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  call 1048746:15 0
  call 1048747:18 0
  call 1048756:15 10
fn 32 contains: alloc panic
  call 1048763:11 31
fn 33 repeat: alloc
  site 1048771:36 1 `push` can make the array grow
  site 1048772:5 1 `raw.concat` creates its result on the heap
  unproved
fn 34 parse_int: alloc panic
  site 1048782:14 1 an array of the characters of a string is created here
  site 1048788:21 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048789:17 2 `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first
  site 1048789:17 2 `-` on `i64` can overflow here: use `-%` or `-|`, or check the range first
  call 1048786:21 23
fn 35 parse_float: none
fn 36 bytes: alloc panic
  site 1048807:17 2 the value of this `as` is not proved to fit in `i64`
  site 1048809:22 2 the value of this `as` is not proved to fit in `u8`
  site 1048809:22 1 `push` can make the array grow
  site 1048811:23 2 the value of this `as` is not proved to fit in `u8`
  site 1048811:23 1 `push` can make the array grow
  site 1048812:23 1 `push` can make the array grow
  site 1048818:23 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048818:23 2 the value of this `as` is not proved to fit in `u8`
  site 1048818:23 1 `push` can make the array grow
  site 1048819:23 1 `push` can make the array grow
  site 1048820:23 1 `push` can make the array grow
  site 1048821:23 1 `push` can make the array grow
  site 1048814:23 2 the value of this `as` is not proved to fit in `u8`
  site 1048814:23 1 `push` can make the array grow
  site 1048815:23 1 `push` can make the array grow
  site 1048816:23 1 `push` can make the array grow
  call 1048806:15 0
fn 37 from_bytes: alloc panic
  site 1048853:13 2 `+` on `usize` can overflow here: use `+%` or `+|`, or check the range first
  site 1048852:9 2 the counter of this loop can overflow `i64`
  site 1048857:17 2 `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first
  site 1048857:17 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 1048861:18 2 this number is not proved to be a character
  site 1048861:18 1 `push` can make the array grow
  site 1048862:9 2 `+` on `usize` can overflow here: use `+%` or `+|`, or check the range first
  call 1048864:10 2
fn 38 constant A: none
fn 39 constant B: none
fn 40 constant B2: none
fn 41 constant C: none
fn 42 constant C2: none
fn 43 constant D: panic
  site 9:14 2 the shift of this `<<` is not proved to be in range
  site 9:14 2 this number is not proved to fit in `i64`
fn 44 constant E: alloc
  site 10:23 1 an array is created here
  site 10:16 1 an array is converted here, which makes a new one
fn 45 constant F: alloc
  site 11:22 1 a string is built here
  site 11:22 1 an array is created here
fn 46 constant G: alloc
  site 12:17 1 a string is built here
fn 47 constant H: none
fn 48 constant I: none
fn 49 constant J: none
fn 50 constant K: none
fn 51 constant L: alloc
  call 17:14 27
fn 52 constant M: none
fn 53 constant N: none
fn 54 constant O: none
fn 55 constant Q: alloc
  site 21:22 1 an array is created here
  site 21:15 1 an array is converted here, which makes a new one
fn 56 constant R: panic
  call 22:14 61
fn 57 constant S: alloc
  site 23:14 1 a string is built here
fn 58 constant T: alloc panic
  site 24:20 1 an array is created here
  site 24:28 1 an array is created here
  site 24:32 1 an array is created here
  site 24:18 1 an array is converted here, which makes a new one
  site 24:18 2 a number inside this value is not proved to fit in its type
fn 59 constant U: panic
  site 25:14 2 `-` on `i64` can overflow here
fn 60 constant V: none
fn 61 sum_to: panic
  site 30:22 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 30:5 2 the counter of this loop can overflow `i64`
fn 62 main: panic
  site 37:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 37:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 37:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 37:21 2 this index is not proved to be in range: compare it with the length first, or use `get`
  site 37:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 37:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 37:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 37:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
  site 37:5 2 `+` on `i64` can overflow here: use `+%` or `+|`, or check the range first
fn 63 comptime: panic
  site 35:22 2 `*` on `i64` can overflow here: use `*%` or `*|`, or check the range first
  call 35:22 61
fn 64 comptime: alloc
  call 36:22 33
