#!/bin/sh
# Writes seed/mouc.c.gz again from the sources in compiler/, with the
# compiler ./mouc (see bootstrap.sh). Run it after changing the compiler
# in a way the old seed could not compile.
set -eu
cd "$(dirname "$0")"
./mouc c compiler/main.mou std src/runtime.c | gzip -9 > seed/mouc.c.gz
echo "seed/mouc.c.gz written"
