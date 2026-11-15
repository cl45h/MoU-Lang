/* Calls the functions of lib.mou, built as a library by `mouc build`. */
#include <stdio.h>
#include "lib.h"

int main(void) {
    printf("%lld %.3f %.3f %zu %d %d\n", (long long)mou_fib(20), mou_scaled(2.0, true), mou_scaled(9.0, false), mou_letters(5), mou_even(4), mou_even(7));
    mou_nothing();
    return 0;
}
