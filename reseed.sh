#!/bin/sh
# Writes seed/mou.c.gz again from the sources in compiler/, with the
# compiler ./mou (see bootstrap.sh). Run it after changing the compiler
# in a way the old seed could not compile.
set -eu
cd "$(dirname "$0")"
./mou c compiler/main.mou std runtime/runtime.c | gzip -9 > seed/mou.c.gz
echo "seed/mou.c.gz written"
