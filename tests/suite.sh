#!/bin/sh
# The tests of the project, with the compiler written in Mou and a C
# compiler: no Rust is needed.
#
#   ./bootstrap.sh && tests/suite.sh        runs everything
#   tests/suite.sh native                   only the parts named: native,
#                                           c, stages, abi, fmt, self
#   tests/suite.sh --accept [parts]         writes what the programs give now
#                                           as what is expected of them
#
# What each program of tests/native and examples prints, and the status
# it ends with, is in tests/expected/native. What each stage of the
# compiler says of the programs with mistakes is in tests/expected/stages.
# The programs of tests/c call C, and what they print is next to them.
set -u
cd "$(dirname "$0")/.."
MOUC="${MOUC:-$PWD/mouc}"
CC="${CC:-cc}"
JOBS="${JOBS:-$(nproc 2>/dev/null || echo 2)}"
export MOU_HOME="$PWD" MOUC CC
export CFLAGS="${CFLAGS:--O1}"
accept=0
if [ "${1:-}" = "--accept" ]; then
    accept=1
    shift
fi
export accept
parts="${*:-native c stages abi fmt self}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export work
if [ ! -x "$MOUC" ]; then
    echo "suite: there is no compiler at $MOUC: run ./bootstrap.sh first" >&2
    exit 2
fi

# Compares what a test gave with what is expected of it, or writes it down.
cat > "$work/settle" <<'END'
#!/bin/sh
# settle <name> <got> <expected>
if [ "$accept" = 1 ]; then
    cp "$2" "$3"
    echo "ok    $1"
elif cmp -s "$2" "$3"; then
    echo "ok    $1"
else
    echo "FAIL  $1"
    diff "$3" "$2" 2>&1 | head -5 | sed 's/^/      /'
fi
END

# A program of tests/native or of examples: what it prints and its status.
cat > "$work/native" <<'END'
#!/bin/sh
f=$1
name=$(basename "$f" .mou)
case $f in examples/*) name=example-$name ;; esac
got="$work/native-$name.out"
if "$MOUC" build "$f" -o "$work/native-$name" > "$work/native-$name.log" 2>&1; then
    "$work/native-$name" < /dev/null > "$got" 2> /dev/null
    echo "status $?" >> "$got"
else
    : > "$got"
    echo "status 1" >> "$got"
fi
"$work/settle" "native $name" "$got" "tests/expected/native/$name.out"
END

# A program of tests/c: it must print what is next to it and end well.
cat > "$work/c" <<'END'
#!/bin/sh
f=$1
name=$(basename "$f" .out)
case $name in
    python|pyimport)
        if ! python3-config --includes > /dev/null 2>&1; then
            echo "skip  c $name (no python3-config)"
            exit 0
        fi ;;
    tls)
        if ! command -v openssl > /dev/null 2>&1; then
            echo "skip  c $name (no openssl)"
            exit 0
        fi ;;
esac
got="$work/c-$name.out"
if "$MOUC" build "tests/c/$name.mou" -o "$work/c-$name" > "$work/c-$name.log" 2>&1; then
    "$work/c-$name" < /dev/null > "$got" 2> /dev/null
    status=$?
    [ "$status" = 0 ] || echo "status $status" >> "$got"
else
    head -3 "$work/c-$name.log" > "$got"
fi
"$work/settle" "c $name" "$got" "tests/c/$name.out"
END

# What a stage of the compiler says of a program with mistakes.
cat > "$work/stages" <<'END'
#!/bin/sh
f=$1
name=$(basename "$(dirname "$f")")-$(basename "$f" .mou)
for stage in decls types lowered fx c; do
    expected="tests/expected/stages/$name.$stage"
    # The C of a program without mistakes is not kept: it runs elsewhere.
    [ -f "$expected" ] || [ "$accept" = 1 ] || continue
    got="$work/stage-$name.$stage"
    "$MOUC" "$stage" "$f" std src/runtime.c > "$got" 2> /dev/null
    if [ "$stage" = c ] && ! head -c 5 "$got" | grep -q '^error'; then
        continue
    fi
    "$work/settle" "$stage $name" "$got" "$expected"
done
END
chmod +x "$work/settle" "$work/native" "$work/c" "$work/stages"

each() { # each <script> <files...>: runs the script on every file, several at a time
    script=$1
    shift
    printf '%s\n' "$@" | xargs -P "$JOBS" -n 1 "$work/$script"
}

for part in $parts; do
    case $part in
        native) each native tests/native/*.mou examples/*.mou ;;
        c) each c tests/c/*.out ;;
        stages) each stages tests/decls/*.mou tests/types/*.mou tests/lower/*.mou tests/own/*.mou tests/fx/*.mou tests/consts/*.mou ;;
        abi)
            # A library: an object and a header, which a program in C uses.
            got="$work/abi.out"
            if "$MOUC" build tests/abi/lib.mou -o "$work/lib" > "$work/abi.log" 2>&1 &&
                "$CC" -O1 -I "$work" -o "$work/caller" tests/abi/caller.c "$work/lib.o" -lm -pthread >> "$work/abi.log" 2>&1; then
                "$work/caller" > "$got" 2>&1
            else
                head -3 "$work/abi.log" > "$got"
            fi
            "$work/settle" "abi" "$got" tests/abi/expected.out
            ;;
        fmt)
            # Every file of the project has the standard layout.
            bad=0
            for f in compiler/*.mou std/*.mou examples/*.mou tests/abi/*.mou; do
                "$MOUC" fmt "$f" --check > /dev/null 2>&1 || {
                    echo "FAIL  fmt $f"
                    bad=1
                }
            done
            [ "$bad" = 1 ] || echo "ok    fmt"
            ;;
        self)
            # The compiler compiles itself, and what comes out does it again.
            "$MOUC" c compiler/main.mou std src/runtime.c > "$work/one.c" 2> /dev/null &&
                "$CC" $CFLAGS -w -o "$work/one" "$work/one.c" -lm &&
                "$work/one" c compiler/main.mou std src/runtime.c > "$work/two.c" 2> /dev/null
            if cmp -s "$work/one.c" "$work/two.c" && [ -s "$work/one.c" ]; then
                echo "ok    self"
            else
                echo "FAIL  self: the compiler does not write itself again"
            fi
            ;;
        *)
            echo "suite: there is no part called \`$part\`" >&2
            exit 2
            ;;
    esac
done > "$work/report"
grep -v '^ok ' "$work/report"
passed=$(grep -c '^ok ' "$work/report")
failed=$(grep -c '^FAIL ' "$work/report")
skipped=$(grep -c '^skip ' "$work/report")
echo "$passed passed, $failed failed, $skipped skipped"
[ "$failed" = 0 ]
