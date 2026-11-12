#!/bin/sh
# Builds the Mou compiler from its own sources, with nothing but a C
# compiler: no Rust is needed.
#
#   ./bootstrap.sh            leaves the compiler in ./mouc
#
# seed/mouc.c.gz is the compiler written in Mou (compiler/*.mou), already
# compiled to C by itself. This script builds that seed, uses it to
# compile the sources that are here now, builds what comes out, and
# checks that this second compiler writes the same C again.
set -eu
cd "$(dirname "$0")"
CC="${CC:-cc}"
CFLAGS="${CFLAGS:--O1}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

gzip -dc seed/mouc.c.gz > "$work/stage0.c"
"$CC" $CFLAGS -w -o "$work/stage0" "$work/stage0.c" -lm
"$work/stage0" c compiler/main.mou std src/runtime.c > "$work/stage1.c"
"$CC" $CFLAGS -w -o "$work/stage1" "$work/stage1.c" -lm
"$work/stage1" c compiler/main.mou std src/runtime.c > "$work/stage2.c"
if ! cmp -s "$work/stage1.c" "$work/stage2.c"; then
    echo "bootstrap: the compiler does not write itself again" >&2
    exit 1
fi
cp "$work/stage1" ./mouc
echo "built ./mouc from the sources in compiler/ (it compiles itself to the same C)"
echo "use: ./mouc c program.mou std src/runtime.c > program.c && $CC -O2 -o program program.c -lm"
