/* Runtime of the C backend. It is pasted at the top of every generated file.
 *
 * Values are tagged and reference counted, exactly like the interpreter's:
 * this first backend favours being correct and portable over being fast.
 * Needs a compiler with `__int128` (GCC or Clang). */
#include <errno.h>
#include <math.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#if defined(__unix__) || defined(__APPLE__)
#define KPOSIX 1
#include <spawn.h>
#include <sys/wait.h>
#include <unistd.h>
extern char **environ;
#endif

/* A program that starts tasks is compiled with KTHREADS: the counts of
   references are then atomic, since a value can be shared between
   threads, and what says where the program is belongs to each thread. */
#ifdef KTHREADS
#include <pthread.h>
#define KLOCAL _Thread_local
#define KRC(o) __atomic_load_n(&(o)->rc, __ATOMIC_RELAXED)
#define KINC(o) __atomic_add_fetch(&(o)->rc, 1, __ATOMIC_RELAXED)
#define KDEC(o) __atomic_sub_fetch(&(o)->rc, 1, __ATOMIC_ACQ_REL)
#else
#define KLOCAL
#define KRC(o) ((o)->rc)
#define KINC(o) (++(o)->rc)
#define KDEC(o) (--(o)->rc)
#endif

typedef __int128 i128;
typedef unsigned __int128 u128;
#define I128_MAX ((i128)(((u128)1 << 127) - 1))
#define I128_MIN (-I128_MAX - 1)

/* `T_INT` is a whole number that fits in 64 bits. One that does not (an
   `i128`, or a `u64` from 2^63 on) is `T_BIG`: a block that holds it. A
   number that fits is never kept as `T_BIG`, so two numbers are equal
   only when their tags are. */
enum { T_UNIT, T_INT, T_F64, T_F32, T_BOOL, T_CHAR, T_STR, T_TUPLE, T_STRUCT, T_ENUM, T_ARRAY, T_NONE, T_SOME, T_OK, T_ERR, T_FN, T_CLOSURE, T_BIG };

/* Header of a heap block; the items (or the bytes of a string) follow it. */
typedef struct kobj {
    int64_t rc;
    int64_t len;
    int64_t cap; /* items there is room for; only arrays grow */
    int32_t pack; /* how an array stores its items */
    uint32_t arena; /* the arena the block was taken from, or 0 */
} kobj;

/* An array whose items are all booleans, integers of 64 bits, floats or
   characters keeps them bare, without their tags. Any other array, and
   every other block, holds tagged values. */
enum { P_NONE, P_BOOL, P_I64, P_F64, P_CHAR, P_REC };

/* An array of structs whose fields are all booleans, integers of up to
   64 bits, `f64` or characters keeps the fields in line, eight bytes
   each, with no block per item: its packing is `P_REC` plus the number of
   the struct. The generated code says which structs allow it: KREC_N is
   the number of fields (0 when the struct does not) and KREC_K the
   packing of each one. */

_Static_assert(sizeof(kobj) % 16 == 0, "the header must not misalign the items");

/* A tagged value: 16 bytes, so that it travels in two registers. */
typedef struct kv {
    uint32_t tag : 8;
    uint32_t a : 24; /* struct index; enum index; function id */
    uint32_t b; /* the variant of an enum */
    union {
        int64_t i;
        double f;
        float g;
        bool t;
        uint32_t c;
        kobj *o;
    } u;
} kv;

_Static_assert(sizeof(kv) == 16, "a tagged value is two words");

#define ITEMS(o) ((kv *)((o) + 1))
#define BYTES(o) ((char *)((o) + 1))

/* Defined by the generated code, after the functions. */
struct kv;
static kv kdispatch(uint32_t id, kv *args, kv env);

/* Where the program is: the line in the high half; in the low one, the
   function (20 bits, an index in KFUNCS) and the column (12 bits). */
static KLOCAL uint64_t kpos;
#define KL(l, c) (kpos = ((uint64_t)(l) << 32) | (uint32_t)(c))

static _Noreturn void kpanic(const char *msg) {
    fflush(stdout);
    /* A line carries the number of its module in the high bits. */
    int kline = (int)(kpos >> 32), kcol = (int)(kpos & 0xFFF);
    const char *kfunc = KFUNCS[(uint32_t)kpos >> 12];
    int file = kline >> KFILE_SHIFT;
    const char *path = file < KFILE_COUNT ? KFILES[file] : KFILE;
    fprintf(stderr, "%s:%d:%d: error: panic: %s\n  in %s\n", path, kline & ((1 << KFILE_SHIFT) - 1), kcol, msg, kfunc);
    exit(101);
}

static _Noreturn void kinternal(const char *what) {
    char buf[200];
    snprintf(buf, sizeof buf, "internal error in the runtime: %s", what);
    kpanic(buf);
}

/* ---- text buffer ---- */

typedef struct {
    char *p;
    size_t len, cap;
} ksb;

static void ksb_put(ksb *sb, const char *s, size_t n) {
    if (sb->len + n + 1 > sb->cap) {
        sb->cap = (sb->len + n + 1) * 2;
        sb->p = (char *)realloc(sb->p, sb->cap);
        if (!sb->p) kpanic("out of memory");
    }
    memcpy(sb->p + sb->len, s, n);
    sb->len += n;
    sb->p[sb->len] = 0;
}

static void ksb_str(ksb *sb, const char *s) { ksb_put(sb, s, strlen(s)); }

static void ksb_i128(ksb *sb, i128 v) {
    char buf[48];
    int n = 48;
    u128 u = v < 0 ? (u128)0 - (u128)v : (u128)v;
    do {
        buf[--n] = (char)('0' + (int)(u % 10));
        u /= 10;
    } while (u);
    if (v < 0) buf[--n] = '-';
    ksb_put(sb, buf + n, (size_t)(48 - n));
}

static void ksb_char(ksb *sb, uint32_t c) {
    char b[4];
    size_t n;
    if (c < 0x80) {
        b[0] = (char)c;
        n = 1;
    } else if (c < 0x800) {
        b[0] = (char)(0xC0 | (c >> 6));
        b[1] = (char)(0x80 | (c & 0x3F));
        n = 2;
    } else if (c < 0x10000) {
        b[0] = (char)(0xE0 | (c >> 12));
        b[1] = (char)(0x80 | ((c >> 6) & 0x3F));
        b[2] = (char)(0x80 | (c & 0x3F));
        n = 3;
    } else {
        b[0] = (char)(0xF0 | (c >> 18));
        b[1] = (char)(0x80 | ((c >> 12) & 0x3F));
        b[2] = (char)(0x80 | ((c >> 6) & 0x3F));
        b[3] = (char)(0x80 | (c & 0x3F));
        n = 4;
    }
    ksb_put(sb, b, n);
}

/* A character the way it appears inside quotes. */
static void ksb_escaped(ksb *sb, uint32_t c, char quote) {
    char buf[16];
    if (c == (uint32_t)quote || c == '\\') {
        buf[0] = '\\';
        buf[1] = (char)c;
        ksb_put(sb, buf, 2);
    } else if (c == '\n') ksb_str(sb, "\\n");
    else if (c == '\r') ksb_str(sb, "\\r");
    else if (c == '\t') ksb_str(sb, "\\t");
    else if (c == 0) ksb_str(sb, "\\0");
    else if (c < 0x20 || c == 0x7F) {
        snprintf(buf, sizeof buf, "\\u{%x}", (unsigned)c);
        ksb_str(sb, buf);
    } else ksb_char(sb, c);
}

/* The shortest decimal text that reads back as the same float. */
static void ksb_float(ksb *sb, double f, bool single) {
    char buf[64], digits[32];
    if (isnan(f)) {
        ksb_str(sb, "NaN");
        return;
    }
    if (isinf(f)) {
        ksb_str(sb, f < 0 ? "-inf" : "inf");
        return;
    }
    for (int prec = 0; prec < 17; prec++) {
        snprintf(buf, sizeof buf, "%.*e", prec, f);
        if (single ? strtof(buf, NULL) == (float)f : strtod(buf, NULL) == f) break;
    }
    /* buf is [-]d[.ddd]e[+-]xx */
    const char *p = buf;
    if (*p == '-') {
        ksb_str(sb, "-");
        p++;
    }
    int nd = 0;
    for (; *p && *p != 'e'; p++)
        if (*p != '.') digits[nd++] = *p;
    int exp = atoi(p + 1);
    while (nd > 1 && digits[nd - 1] == '0') nd--;
    double mag = fabs(f);
    if (mag != 0 && (mag >= 1e16 || mag < 1e-4)) {
        ksb_put(sb, digits, 1);
        if (nd > 1) {
            ksb_str(sb, ".");
            ksb_put(sb, digits + 1, (size_t)(nd - 1));
        }
        snprintf(buf, sizeof buf, "e%d", exp);
        ksb_str(sb, buf);
    } else if (exp < 0) {
        ksb_str(sb, "0.");
        for (int i = -1; i > exp; i--) ksb_str(sb, "0");
        ksb_put(sb, digits, (size_t)nd);
    } else {
        for (int i = 0; i <= exp; i++) ksb_put(sb, i < nd ? digits + i : "0", 1);
        ksb_str(sb, ".");
        if (nd > exp + 1) ksb_put(sb, digits + exp + 1, (size_t)(nd - exp - 1));
        else ksb_str(sb, "0");
    }
}

/* ---- values ---- */

static inline kv kmk(int tag) {
    kv v = {0};
    v.tag = (uint8_t)tag;
    return v;
}

#define KUNIT kmk(T_UNIT)
#define KNONE kmk(T_NONE)

static inline kv ki64(int64_t i) {
    kv v = kmk(T_INT);
    v.u.i = i;
    return v;
}

static kv kbig(i128 i);

static kv kint(i128 i) {
    if (i >= INT64_MIN && i <= INT64_MAX) return ki64((int64_t)i);
    return kbig(i);
}

#define KISINT(v) ((v).tag == T_INT || (v).tag == T_BIG)

/* The whole number a value is. The value is only looked at. */
static inline i128 kwide(kv v) {
    if (v.tag == T_INT) return v.u.i;
    i128 w;
    memcpy(&w, v.u.o + 1, sizeof w);
    return w;
}

static void krelease(kv v);

/* The whole number a value is, letting go of the value. */
static inline i128 ktakei(kv v) {
    if (v.tag == T_INT) return v.u.i;
    i128 w = kwide(v);
    krelease(v);
    return w;
}

static kv kint2(uint64_t hi, uint64_t lo) { return kint((i128)(((u128)hi << 64) | lo)); }

static kv kf64(double f) {
    kv v = kmk(T_F64);
    v.u.f = f;
    return v;
}

static kv kf64_bits(uint64_t bits) {
    double f;
    memcpy(&f, &bits, sizeof f);
    return kf64(f);
}

static kv kf32(float g) {
    kv v = kmk(T_F32);
    v.u.g = g;
    return v;
}

static kv kbool(bool t) {
    kv v = kmk(T_BOOL);
    v.u.t = t;
    return v;
}

static kv kchar(uint32_t c) {
    kv v = kmk(T_CHAR);
    v.u.c = c;
    return v;
}

static kv kfn(uint32_t id) {
    kv v = kmk(T_FN);
    v.a = id;
    return v;
}

/* The blocks of structs, enums, tuples and the like are small and die
   young: one that is let go is kept, by its number of items, for the next
   one of that size, instead of going back to `malloc`. Each thread keeps
   its own. Under the address sanitizer every block is given back, so that
   it can see a block used after it was let go. */
#if defined(__SANITIZE_ADDRESS__) || defined(KNOPOOL)
#define KPOOL_MAX 0
#else
#define KPOOL_MAX 8
#endif
static KLOCAL kobj *kpool[KPOOL_MAX + 1];
#define KPOOL_NEXT(o) (*(kobj **)((o) + 1))

static void kpool_drop(void) {
    for (int n = 0; n <= KPOOL_MAX; n++) {
        while (kpool[n]) {
            kobj *o = kpool[n];
            kpool[n] = KPOOL_NEXT(o);
            free(o);
        }
    }
}

/* ---- arenas ----
 *
 * Inside `with alloc = arena { }` a block is taken from the end of a big
 * chunk instead of asked from `malloc`, and all of them go back at once.
 * A block made there can outlive the `with`: the arena counts its blocks
 * still in use and gives its chunks back when the `with` is over and the
 * last of them is let go. So an arena changes how memory is asked for,
 * never what a program does.
 *
 * A program that starts tasks does not use arenas (a block could be let
 * go by another thread), and neither does one built with the address
 * sanitizer, which then sees every block. */
#if defined(__SANITIZE_ADDRESS__) || defined(KTHREADS) || defined(KNOARENA)
#define KARENAS 0
#else
#define KARENAS 1
#endif
typedef struct kchunk {
    struct kchunk *next;
    size_t used, cap, pad;
} kchunk;
typedef struct karena {
    kchunk *chunks;
    int64_t live; /* blocks taken and not let go yet */
    bool open; /* still inside its `with` */
    uint32_t outer; /* the arena in use when it was opened */
} karena;
enum { KARENA_MAX = 1024, KCHUNK = 1 << 16, KCHUNK_KEPT = 256 };
static karena *karenas[KARENA_MAX];
static uint32_t karena_cur;
/* Chunks of the usual size that an arena gave back, for the next one. */
static kchunk *kchunk_spare;
static int kchunk_spares;

static void *karena_take(karena *a, size_t bytes) {
    bytes = (bytes + 15) & ~(size_t)15;
    kchunk *c = a->chunks;
    if (!c || c->used + bytes > c->cap) {
        size_t cap = bytes > KCHUNK ? bytes : KCHUNK;
        if (cap == KCHUNK && kchunk_spare) {
            c = kchunk_spare;
            kchunk_spare = c->next;
            kchunk_spares--;
        } else {
            c = (kchunk *)malloc(sizeof(kchunk) + cap);
            if (!c) kpanic("out of memory");
        }
        c->next = a->chunks;
        c->used = 0;
        c->cap = cap;
        a->chunks = c;
    }
    void *p = (char *)(c + 1) + c->used;
    c->used += bytes;
    return p;
}

static void karena_drop(uint32_t id) {
    karena *a = karenas[id];
    while (a->chunks) {
        kchunk *c = a->chunks;
        a->chunks = c->next;
        if (c->cap == KCHUNK && kchunk_spares < KCHUNK_KEPT) {
            c->next = kchunk_spare;
            kchunk_spare = c;
            kchunk_spares++;
        } else {
            free(c);
        }
    }
    free(a);
    karenas[id] = NULL;
}

/* Gives back the memory of a block nothing uses. */
static void kfree_obj(kobj *o) {
    if (o->arena) {
        karena *a = karenas[o->arena];
        if (--a->live == 0 && !a->open) karena_drop(o->arena);
    } else {
        free(o);
    }
}

static kv kint(i128 i);

static kv karena_open(void) {
    if (!KARENAS) return kint(0);
    for (uint32_t id = 1; id < KARENA_MAX; id++) {
        if (karenas[id]) continue;
        karena *a = (karena *)calloc(1, sizeof(karena));
        if (!a) kpanic("out of memory");
        a->open = true;
        a->outer = karena_cur;
        karenas[id] = a;
        karena_cur = id;
        return kint(id);
    }
    /* Too many at once: this one asks `malloc` as usual. */
    return kint(0);
}

static bool kheap(kv v) { return v.tag >= T_STR && v.tag != T_NONE && v.tag != T_FN && v.u.o != NULL; }

static kv kretain(kv v) {
    if (kheap(v)) KINC(v.u.o);
    return v;
}

static void krelease(kv v) {
    if (!kheap(v) || KDEC(v.u.o) > 0) return;
    if (v.tag != T_STR && v.u.o->pack == P_NONE)
        for (int64_t i = 0; i < v.u.o->len; i++) krelease(ITEMS(v.u.o)[i]);
    kobj *o = v.u.o;
    if (!o->arena && v.tag != T_STR && v.tag != T_ARRAY && o->len > 0 && o->len <= KPOOL_MAX) {
        int64_t n = o->len;
        KPOOL_NEXT(o) = kpool[n];
        kpool[n] = o;
        return;
    }
    kfree_obj(o);
}

/* Lets go of the locals of a function that are listed. */
static void kdrop(kv *locals, const int *at, int n) {
    for (int i = 0; i < n; i++) {
        kv v = locals[at[i]];
        if (v.tag >= T_STR) krelease(v);
    }
}

static void kset(kv *slot, kv v) {
    kv old = *slot;
    *slot = v;
    krelease(old);
}

static kobj *kalloc(size_t bytes, int64_t len) {
    kobj *o;
    if (karena_cur) {
        karena *a = karenas[karena_cur];
        o = (kobj *)karena_take(a, sizeof(kobj) + bytes);
        a->live++;
        o->arena = karena_cur;
    } else {
        o = (kobj *)malloc(sizeof(kobj) + bytes);
        if (!o) kpanic("out of memory");
        o->arena = 0;
    }
    o->rc = 1;
    o->len = len;
    o->cap = len;
    o->pack = P_NONE;
    return o;
}

/* A whole number that does not fit in 64 bits: a block with no items,
   and the number after its header. */
static kv kbig(i128 i) {
    kv v = kmk(T_BIG);
    v.u.o = kalloc(sizeof i, 0);
    memcpy(v.u.o + 1, &i, sizeof i);
    return v;
}

/* ---- packed arrays ---- */

static const size_t KPACK_SIZES[] = {sizeof(kv), 1, 8, 8, 4};

/* Bytes an item takes. */
static inline size_t kpack_size(int64_t pack) {
    return pack < P_REC ? KPACK_SIZES[pack] : (size_t)KREC_N[pack - P_REC] * 8;
}

/* The packing that can hold `v` bare. */
static int kpack_of(kv v) {
    switch (v.tag) {
    case T_BOOL: return P_BOOL;
    case T_INT: return P_I64;
    case T_F64: return P_F64;
    case T_CHAR: return P_CHAR;
    case T_STRUCT: return v.a < KREC_COUNT && KREC_N[v.a] > 0 ? (int)(P_REC + v.a) : P_NONE;
    default: return P_NONE;
    }
}

/* How a field is kept, besides the packings of single values: an integer
   without sign of 64 bits, whose slot holds its bits. */
enum { KF_U64 = 5 };

/* Field `f` of item `i` of an array of structs kept in line. */
static inline kv krec_field(const kobj *o, int64_t i, int64_t f) {
    int s = (int)(o->pack - P_REC);
    const int64_t *slot = (const int64_t *)(o + 1) + i * KREC_N[s] + f;
    kv v;
    switch (KREC_K[s][f]) {
    case P_BOOL: v = kmk(T_BOOL); v.u.t = *slot != 0; return v;
    case P_F64: v = kmk(T_F64); memcpy(&v.u.f, slot, 8); return v;
    case P_CHAR: v = kmk(T_CHAR); v.u.c = (uint32_t)*slot; return v;
    case KF_U64: return kint((i128)(uint64_t)*slot);
    default: v = kmk(T_INT); v.u.i = *slot; return v;
    }
}

static inline void krec_store(kobj *o, int64_t i, int64_t f, kv v) {
    int s = (int)(o->pack - P_REC);
    int64_t *slot = (int64_t *)(o + 1) + i * KREC_N[s] + f;
    switch (KREC_K[s][f]) {
    case P_BOOL: *slot = v.u.t; break;
    case P_F64: memcpy(slot, &v.u.f, 8); break;
    case P_CHAR: *slot = v.u.c; break;
    default: *slot = (int64_t)kwide(v); break;
    }
}

static kv kagg(int tag, uint32_t a, uint32_t b, int64_t n, const kv *items);

/* Item `i` of such an array as a struct of its own. */
static kv krec_box(const kobj *o, int64_t i) {
    int s = (int)(o->pack - P_REC), n = KREC_N[s];
    kv fields[n];
    for (int f = 0; f < n; f++) fields[f] = krec_field(o, i, f);
    return kagg(T_STRUCT, (uint32_t)s, 0, n, fields);
}

/* Item `i` of a block that does not keep structs in line. An item of a
   packed array owns nothing, so the result is only borrowed either way. */
static kv kelem(const kobj *o, int64_t i) {
    kv v;
    switch (o->pack) {
    case P_BOOL: v = kmk(T_BOOL); v.u.t = ((const uint8_t *)(o + 1))[i]; return v;
    case P_I64: v = kmk(T_INT); v.u.i = ((const int64_t *)(o + 1))[i]; return v;
    case P_F64: v = kmk(T_F64); v.u.f = ((const double *)(o + 1))[i]; return v;
    case P_CHAR: v = kmk(T_CHAR); v.u.c = ((const uint32_t *)(o + 1))[i]; return v;
    default: return ((const kv *)(o + 1))[i];
    }
}

static void krelease(kv v);

/* Item `i` of a block, for the caller to own. */
static kv kown(const kobj *o, int64_t i) {
    if (o->pack >= P_REC) return krec_box(o, i);
    return kretain(kelem(o, i));
}

/* Writes an item the packing of the block can hold, taking ownership. */
static void kraw(kobj *o, int64_t i, kv v) {
    if (o->pack >= P_REC) {
        for (int f = 0; f < KREC_N[o->pack - P_REC]; f++) krec_store(o, i, f, ITEMS(v.u.o)[f]);
        krelease(v);
        return;
    }
    switch (o->pack) {
    case P_BOOL: ((uint8_t *)(o + 1))[i] = v.u.t; break;
    case P_I64: ((int64_t *)(o + 1))[i] = (int64_t)v.u.i; break;
    case P_F64: ((double *)(o + 1))[i] = v.u.f; break;
    case P_CHAR: ((uint32_t *)(o + 1))[i] = v.u.c; break;
    default: ((kv *)(o + 1))[i] = v; break;
    }
}

/* An array with room for `len` items of the given packing, all unset. */
static kv karray_new(int pack, int64_t len) {
    kv v = kmk(T_ARRAY);
    v.u.o = kalloc((size_t)len * kpack_size(pack), len);
    v.u.o->pack = pack;
    return v;
}

/* Gives the array of `v` tagged items, so that it can hold any value. */
static void kunpack(kv *v) {
    kobj *o = v->u.o;
    if (o->pack == P_NONE) return;
    kobj *n = kalloc((size_t)o->len * sizeof(kv), o->len);
    for (int64_t i = 0; i < o->len; i++) ITEMS(n)[i] = o->pack >= P_REC ? krec_box(o, i) : kelem(o, i);
    if (KDEC(o) == 0) kfree_obj(o);
    v->u.o = n;
}

/* Packs an array of tagged items that all allow the same packing. The
   array must not be shared. */
static void krepack(kv *v) {
    kobj *o = v->u.o;
    if (o->pack != P_NONE || o->len == 0) return;
    int pack = kpack_of(ITEMS(o)[0]);
    for (int64_t i = 1; i < o->len && pack != P_NONE; i++)
        if (kpack_of(ITEMS(o)[i]) != pack) pack = P_NONE;
    if (pack == P_NONE) return;
    kobj *n = kalloc((size_t)o->len * kpack_size(pack), o->len);
    n->pack = pack;
    for (int64_t i = 0; i < o->len; i++) kraw(n, i, ITEMS(o)[i]);
    kfree_obj(o);
    v->u.o = n;
}

/* A block of `n` items, taking ownership of them. */
static kv kagg(int tag, uint32_t a, uint32_t b, int64_t n, const kv *items) {
    kv v = kmk(tag);
    v.a = a;
    v.b = b;
    if (!karena_cur && tag != T_ARRAY && n > 0 && n <= KPOOL_MAX && kpool[n]) {
        kobj *o = kpool[n];
        kpool[n] = KPOOL_NEXT(o);
        o->rc = 1;
        o->len = n;
        o->cap = n;
        o->pack = P_NONE;
        memcpy(ITEMS(o), items, (size_t)n * sizeof(kv));
        v.u.o = o;
        return v;
    }
    if (n > 0 || tag == T_ARRAY) {
        v.u.o = kalloc((size_t)n * sizeof(kv), n);
        if (n > 0) memcpy(ITEMS(v.u.o), items, (size_t)n * sizeof(kv));
        if (tag == T_ARRAY) krepack(&v);
    }
    return v;
}

static kv kbox(int tag, kv item) { return kagg(tag, 0, 0, 1, &item); }

static kv kstr(const char *s, size_t n) {
    kv v = kmk(T_STR);
    v.u.o = kalloc(n + 1, (int64_t)n);
    memcpy(BYTES(v.u.o), s, n);
    BYTES(v.u.o)[n] = 0;
    return v;
}

static kv ksb_done(ksb *sb) {
    kv v = kstr(sb->p ? sb->p : "", sb->len);
    free(sb->p);
    return v;
}

static int64_t klen(kv v) { return v.u.o ? v.u.o->len : 0; }

/* Makes the block of `v` its own, so that it can be written to. */
static void kunique(kv *v) {
    kobj *o = v->u.o;
    if (!o || KRC(o) == 1) return;
    kobj *n = kalloc((size_t)o->len * kpack_size(o->pack), o->len);
    n->pack = o->pack;
    if (o->pack != P_NONE) memcpy(n + 1, o + 1, (size_t)o->len * kpack_size(o->pack));
    else
        for (int64_t i = 0; i < o->len; i++) ITEMS(n)[i] = kretain(ITEMS(o)[i]);
    /* Another thread may have let go of the block meanwhile. */
    kv old = *v;
    v->u.o = n;
    krelease(old);
}

/* ---- display ---- */

static void kshow(ksb *sb, kv v, bool top) {
    switch (v.tag) {
    case T_UNIT: ksb_str(sb, "()"); break;
    case T_INT: ksb_i128(sb, v.u.i); break;
    case T_BIG: ksb_i128(sb, kwide(v)); break;
    case T_F64: ksb_float(sb, v.u.f, false); break;
    case T_F32: ksb_float(sb, (double)v.u.g, true); break;
    case T_BOOL: ksb_str(sb, v.u.t ? "true" : "false"); break;
    case T_CHAR:
        if (top) ksb_char(sb, v.u.c);
        else {
            ksb_str(sb, "'");
            ksb_escaped(sb, v.u.c, '\'');
            ksb_str(sb, "'");
        }
        break;
    case T_STR:
        if (top) ksb_put(sb, BYTES(v.u.o), (size_t)v.u.o->len);
        else {
            ksb_str(sb, "\"");
            const unsigned char *s = (const unsigned char *)BYTES(v.u.o);
            for (int64_t i = 0; i < v.u.o->len;) {
                uint32_t c = s[i];
                int n = c < 0x80 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4;
                if (n == 1) ksb_escaped(sb, c, '"');
                else ksb_put(sb, (const char *)s + i, (size_t)n);
                i += n;
            }
            ksb_str(sb, "\"");
        }
        break;
    case T_TUPLE:
    case T_ARRAY:
        ksb_str(sb, v.tag == T_TUPLE ? "(" : "[");
        for (int64_t i = 0; i < klen(v); i++) {
            if (i) ksb_str(sb, ", ");
            kv item = kown(v.u.o, i);
            kshow(sb, item, false);
            krelease(item);
        }
        ksb_str(sb, v.tag == T_TUPLE ? ")" : "]");
        break;
    case T_STRUCT:
        /* A `dyn Trait` value is shown as the value it holds. */
        if (strncmp(KSTRUCT_NAME[v.a], "dyn ", 4) == 0 && klen(v) > 0) {
            kshow(sb, ITEMS(v.u.o)[0], top);
            break;
        }
        ksb_str(sb, KSTRUCT_NAME[v.a]);
        ksb_str(sb, "(");
        for (int64_t i = 0; i < klen(v); i++) {
            if (i) ksb_str(sb, ", ");
            ksb_str(sb, KSTRUCT_FIELDS[v.a][i]);
            ksb_str(sb, ": ");
            kshow(sb, ITEMS(v.u.o)[i], false);
        }
        ksb_str(sb, ")");
        break;
    case T_ENUM:
        ksb_str(sb, KENUM_VARIANTS[v.a][v.b]);
        if (klen(v) == 0) break;
        ksb_str(sb, "(");
        for (int64_t i = 0; i < klen(v); i++) {
            if (i) ksb_str(sb, ", ");
            if (KENUM_FIELDS[v.a][v.b][i]) {
                ksb_str(sb, KENUM_FIELDS[v.a][v.b][i]);
                ksb_str(sb, ": ");
            }
            kshow(sb, ITEMS(v.u.o)[i], false);
        }
        ksb_str(sb, ")");
        break;
    case T_NONE: ksb_str(sb, "None"); break;
    case T_SOME:
        ksb_str(sb, "Some(");
        kshow(sb, ITEMS(v.u.o)[0], false);
        ksb_str(sb, ")");
        break;
    case T_OK: kshow(sb, ITEMS(v.u.o)[0], top); break;
    case T_ERR:
        ksb_str(sb, "error ");
        kshow(sb, ITEMS(v.u.o)[0], false);
        break;
    default:
        ksb_str(sb, "fn ");
        ksb_str(sb, KFN_NAME[v.a]);
    }
}

static void ksb_lit(ksb *sb, const char *s, size_t n) { ksb_put(sb, s, n); }

static void ksb_val(ksb *sb, kv v) {
    kshow(sb, v, true);
    krelease(v);
}

static void kpanicf(const char *before, kv v, const char *after) {
    ksb sb = {0};
    ksb_str(&sb, before);
    kshow(&sb, v, false);
    ksb_str(&sb, after);
    kpanic(sb.p);
}

/* ---- places ---- */

typedef struct {
    int index; /* 0: field, 1: element */
    int64_t k;
} kstep;

/* The position an index operand denotes. */
static int64_t kidx(const kv *v) {
    if (!KISINT(*v)) kinternal("an index that is not an integer");
    if (kwide(*v) < 0) kpanicf("index ", *v, " is negative");
    /* One that does not fit is past the end of any array. */
    return v->tag == T_BIG ? INT64_MAX : v->u.i;
}

static int64_t kidxc(i128 i) {
    if (i < 0) kpanicf("index ", kint(i), " is negative");
    return i > INT64_MAX ? INT64_MAX : (int64_t)i;
}

/* Stops the program unless `k` is a position of the array `v`. */
static void kcheck_index(const kv *v, int64_t k) {
    if (v->tag != T_ARRAY) kinternal("a projection on the wrong kind of value");
    if (k >= klen(*v)) {
        char buf[120];
        snprintf(buf, sizeof buf, "index %lld is out of range for length %lld", (long long)k, (long long)klen(*v));
        kpanic(buf);
    }
}

static kv *kstepto(kv *v, kstep st, bool write) {
    if (st.index) {
        kcheck_index(v, st.k);
        /* A bare item has no address as a tagged value. */
        if (v->u.o->pack != P_NONE) kunpack(v);
    } else if (v->tag < T_TUPLE || v->tag == T_ARRAY || v->tag == T_NONE || v->tag >= T_FN || st.k >= klen(*v)) {
        kinternal("a field that does not exist");
    }
    if (write) kunique(v);
    return &ITEMS(v->u.o)[st.k];
}

/* The array a place holds, made writable. */
static kv *karray_at(kv *root, int n, const kstep *steps, const char *what) {
    kv *v = root;
    for (int i = 0; i < n; i++) v = kstepto(v, steps[i], true);
    if (v->tag != T_ARRAY) kinternal(what);
    kunique(v);
    return v;
}

static kv kpush(kv *root, int n, const kstep *steps, kv item) {
    kv *v = karray_at(root, n, steps, "`push` on a value that is not an array");
    kobj *o = v->u.o;
    int pack = kpack_of(item);
    if (o->len == 0 && o->pack != pack) {
        /* An empty array takes the packing of its first item. */
        o->pack = pack;
        o->cap = 0;
    } else if (o->pack != P_NONE && o->pack != pack) {
        kunpack(v);
        o = v->u.o;
    }
    if (o->len == o->cap) {
        int64_t cap = o->cap < 4 ? 4 : o->cap * 2;
        if (o->arena || karena_cur) {
            /* A block of an arena does not grow where it is, and one that
               grows inside an arena moves into it. */
            size_t size = kpack_size(o->pack);
            kobj *n = kalloc((size_t)cap * size, o->len);
            n->pack = o->pack;
            memcpy(n + 1, o + 1, (size_t)o->len * size);
            kfree_obj(o);
            o = n;
        } else {
            o = (kobj *)realloc(o, sizeof(kobj) + (size_t)cap * kpack_size(o->pack));
            if (!o) kpanic("out of memory");
        }
        o->cap = cap;
        v->u.o = o;
    }
    kraw(o, o->len++, item);
    return kmk(T_UNIT);
}

static kv kpop(kv *root, int n, const kstep *steps) {
    kv *v = karray_at(root, n, steps, "`pop` on a value that is not an array");
    kobj *o = v->u.o;
    if (o->len == 0) return kmk(T_NONE);
    /* The array gives up the item: it is not retained. */
    o->len--;
    kv item = o->pack >= P_REC ? krec_box(o, o->len) : kelem(o, o->len);
    return kagg(T_SOME, 0, 0, 1, &item);
}

static kv kget(kv a, kv i) {
    if (a.tag != T_ARRAY || !KISINT(i)) kinternal("`get` on a value that is not an array");
    kv r = kmk(T_NONE);
    if (i.tag == T_INT && i.u.i >= 0 && i.u.i < klen(a)) {
        kv item = kown(a.u.o, (int64_t)i.u.i);
        r = kagg(T_SOME, 0, 0, 1, &item);
    }
    krelease(i);
    krelease(a);
    return r;
}

/* `pop` and `get` of an array of numbers or booleans, for whoever keeps
   the optional without a block: whether there is an item, and the item. */
/* An item handed out bare: a number kept in a block (a `u64` from 2^63
   on) goes as its 64 bits, which is what whoever asked reads. */
static inline void kraw_number(kv *item, bool owned) {
    if (item->tag != T_BIG) return;
    kv big = *item;
    *item = ki64((int64_t)(uint64_t)kwide(big));
    if (owned) krelease(big);
}

static bool kpop_raw(kv *root, int n, const kstep *steps, kv *item) {
    kv *v = karray_at(root, n, steps, "`pop` on a value that is not an array");
    kobj *o = v->u.o;
    if (o->len == 0) return false;
    o->len--;
    *item = kelem(o, o->len);
    kraw_number(item, true);
    return true;
}

static bool kget_raw(kv a, kv i, kv *item) {
    if (a.tag != T_ARRAY || !KISINT(i)) kinternal("`get` on a value that is not an array");
    bool has = i.tag == T_INT && i.u.i >= 0 && i.u.i < klen(a);
    if (has) {
        *item = kelem(a.u.o, (int64_t)i.u.i);
        kraw_number(item, false);
    }
    krelease(i);
    krelease(a);
    return has;
}

/* Puts `value` at position `k` of the array `v`, which is in range. */
static void kitem_put(kv *v, int64_t k, kv value) {
    kunique(v);
    if (v->u.o->pack != P_NONE) {
        if (kpack_of(value) == v->u.o->pack) {
            kraw(v->u.o, k, value);
            return;
        }
        kunpack(v);
    }
    kset(&ITEMS(v->u.o)[k], value);
}

static kv kread(kv *root, int n, const kstep *steps) {
    kv *v = root;
    for (int i = 0; i + 1 < n; i++) v = kstepto(v, steps[i], false);
    if (n > 0 && steps[n - 1].index) {
        kcheck_index(v, steps[n - 1].k);
        return kown(v->u.o, steps[n - 1].k);
    }
    if (n > 0) v = kstepto(v, steps[n - 1], false);
    return kretain(*v);
}

static void kwrite(kv *root, int n, const kstep *steps, kv value) {
    kv *v = root;
    for (int i = 0; i + 1 < n; i++) v = kstepto(v, steps[i], true);
    if (n > 0 && steps[n - 1].index) {
        kcheck_index(v, steps[n - 1].k);
        kitem_put(v, steps[n - 1].k, value);
        return;
    }
    if (n > 0) v = kstepto(v, steps[n - 1], true);
    kset(v, value);
}

/* ---- fast paths of the generated code ---- */

#define KRARE(c) __builtin_expect(!!(c), 0)

static __attribute__((noinline, cold)) void kat_fail(const kv *v, i128 i) {
    kstep st = {1, kidxc(i)};
    kcheck_index(v, st.k);
    kinternal("an index that was wrongly refused");
}

/* The position `i` of the array `v`, checked. */
static inline int64_t kat(const kv *v, i128 i) {
    if (KRARE(v->tag != T_ARRAY || i < 0 || i >= v->u.o->len)) kat_fail(v, i);
    return (int64_t)i;
}

static __attribute__((noinline, cold)) void kfield_fail(void) { kinternal("a field that does not exist"); }

/* Field `k` of a struct, tuple, variant, optional or result. */
static inline kv *kfield(kv *v, int64_t k, bool write) {
    if (KRARE(v->tag < T_TUPLE || v->tag == T_ARRAY || v->tag == T_NONE || v->tag >= T_FN || k >= klen(*v))) kfield_fail();
    if (write && KRARE(KRC(v->u.o) != 1)) kunique(v);
    return &ITEMS(v->u.o)[k];
}

/* Takes the value out of a place for the time of a call, which puts it back. */
static inline kv ktake(kv *v) {
    kv out = *v;
    *v = KUNIT;
    return out;
}

/* Item `i` of an array of tagged values. */
static inline kv *kitemp(kv *v, i128 i, bool write) {
    int64_t k = kat(v, i);
    if (write && KRARE(KRC(v->u.o) != 1)) kunique(v);
    if (KRARE(v->u.o->pack != P_NONE)) kunpack(v);
    return &ITEMS(v->u.o)[k];
}

static inline i128 kitem_int(const kv *v, i128 i) {
    int64_t k = kat(v, i);
    const kobj *o = v->u.o;
    if (KRARE(o->pack != P_I64)) return kwide(kelem(o, k));
    return ((const int64_t *)(o + 1))[k];
}

static inline bool kitem_bool(const kv *v, i128 i) {
    int64_t k = kat(v, i);
    const kobj *o = v->u.o;
    if (KRARE(o->pack != P_BOOL)) return kelem(o, k).u.t;
    return ((const uint8_t *)(o + 1))[k];
}

static inline kv kitem(const kv *v, i128 i) { return kown(v->u.o, kat(v, i)); }

/* ---- fields of the items of an array ----

   `xs[i].f` without making a struct of the item when the array keeps its
   structs in line. */

static __attribute__((noinline, cold)) kv *kitem_field_slow(kv *v, int64_t k, int64_t f, bool write) {
    if (v->u.o->pack != P_NONE) kunpack(v);
    return kfield(&ITEMS(v->u.o)[k], f, write);
}

static inline kv kitem_field(kv *v, i128 i, int64_t f) {
    int64_t k = kat(v, i);
    if (v->u.o->pack >= P_REC) return krec_field(v->u.o, k, f);
    return kretain(*kitem_field_slow(v, k, f, false));
}

static inline void kitem_field_set(kv *v, i128 i, int64_t f, kv value) {
    int64_t k = kat(v, i);
    if (KRARE(KRC(v->u.o) != 1)) kunique(v);
    if (v->u.o->pack >= P_REC) {
        krec_store(v->u.o, k, f, value);
        krelease(value);
    } else kset(kitem_field_slow(v, k, f, true), value);
}

static inline double kitem_field_f64(kv *v, i128 i, int64_t f) {
    int64_t k = kat(v, i);
    const kobj *o = v->u.o;
    if (KRARE(o->pack < P_REC)) return kitem_field_slow(v, k, f, false)->u.f;
    return ((const double *)(o + 1))[k * KREC_N[o->pack - P_REC] + f];
}

static inline i128 kitem_field_int(kv *v, i128 i, int64_t f) {
    int64_t k = kat(v, i);
    const kobj *o = v->u.o;
    if (KRARE(o->pack < P_REC)) return kwide(*kitem_field_slow(v, k, f, false));
    int64_t bits = ((const int64_t *)(o + 1))[k * KREC_N[o->pack - P_REC] + f];
    return KREC_K[o->pack - P_REC][f] == KF_U64 ? (i128)(uint64_t)bits : (i128)bits;
}

static inline bool kitem_field_bool(kv *v, i128 i, int64_t f) {
    int64_t k = kat(v, i);
    const kobj *o = v->u.o;
    if (KRARE(o->pack < P_REC)) return kitem_field_slow(v, k, f, false)->u.t;
    return ((const int64_t *)(o + 1))[k * KREC_N[o->pack - P_REC] + f] != 0;
}

static inline void kitem_field_set_f64(kv *v, i128 i, int64_t f, double x) {
    int64_t k = kat(v, i);
    kobj *o = v->u.o;
    if (KRARE(KRC(o) != 1 || o->pack < P_REC)) kitem_field_set(v, i, f, kf64(x));
    else ((double *)(o + 1))[k * KREC_N[o->pack - P_REC] + f] = x;
}

static inline void kitem_field_set_int(kv *v, i128 i, int64_t f, i128 x) {
    int64_t k = kat(v, i);
    kobj *o = v->u.o;
    if (KRARE(KRC(o) != 1 || o->pack < P_REC)) kitem_field_set(v, i, f, kint(x));
    else ((int64_t *)(o + 1))[k * KREC_N[o->pack - P_REC] + f] = (int64_t)x;
}

static inline void kitem_field_set_bool(kv *v, i128 i, int64_t f, bool t) {
    int64_t k = kat(v, i);
    kobj *o = v->u.o;
    if (KRARE(KRC(o) != 1 || o->pack < P_REC)) kitem_field_set(v, i, f, kbool(t));
    else ((int64_t *)(o + 1))[k * KREC_N[o->pack - P_REC] + f] = t;
}

static inline void kitem_set(kv *v, i128 i, kv value) { kitem_put(v, kat(v, i), value); }

static inline void kitem_set_int(kv *v, i128 i, i128 x) {
    int64_t k = kat(v, i);
    kobj *o = v->u.o;
    if (KRARE(KRC(o) != 1 || o->pack != P_I64 || x != (int64_t)x)) kitem_put(v, k, kint(x));
    else ((int64_t *)(o + 1))[k] = (int64_t)x;
}

static inline void kitem_set_bool(kv *v, i128 i, bool t) {
    int64_t k = kat(v, i);
    kobj *o = v->u.o;
    if (KRARE(KRC(o) != 1 || o->pack != P_BOOL)) kitem_put(v, k, kbool(t));
    else ((uint8_t *)(o + 1))[k] = t;
}

static inline kv kpush_bool(kv *v, bool t) {
    kobj *o = v->u.o;
    if (KRARE(v->tag != T_ARRAY || KRC(o) != 1 || o->pack != P_BOOL || o->len == o->cap)) return kpush(v, 0, NULL, kbool(t));
    ((uint8_t *)(o + 1))[o->len++] = t;
    return kmk(T_UNIT);
}

static inline kv kpush_int(kv *v, i128 x) {
    kobj *o = v->u.o;
    if (KRARE(v->tag != T_ARRAY || KRC(o) != 1 || o->pack != P_I64 || o->len == o->cap || x != (int64_t)x)) return kpush(v, 0, NULL, kint(x));
    ((int64_t *)(o + 1))[o->len++] = (int64_t)x;
    return kmk(T_UNIT);
}

static inline double kdbl(uint64_t bits) {
    double f;
    memcpy(&f, &bits, sizeof f);
    return f;
}

static inline double kitem_f64(const kv *v, i128 i) {
    int64_t k = kat(v, i);
    const kobj *o = v->u.o;
    if (KRARE(o->pack != P_F64)) return kelem(o, k).u.f;
    return ((const double *)(o + 1))[k];
}

static inline void kitem_set_f64(kv *v, i128 i, double f) {
    int64_t k = kat(v, i);
    kobj *o = v->u.o;
    if (KRARE(KRC(o) != 1 || o->pack != P_F64)) kitem_put(v, k, kf64(f));
    else ((double *)(o + 1))[k] = f;
}

static inline kv kpush_f64(kv *v, double f) {
    kobj *o = v->u.o;
    if (KRARE(v->tag != T_ARRAY || KRC(o) != 1 || o->pack != P_F64 || o->len == o->cap)) return kpush(v, 0, NULL, kf64(f));
    ((double *)(o + 1))[o->len++] = f;
    return kmk(T_UNIT);
}

/* ---- places a `ref mut` function computed ----

   The steps from a value to a part of it: an array of pairs, each `0` and
   a field or `1` and an index. */

static int64_t kpath_len(const kv *path) { return klen(*path) / 2; }

static kv *kpath_step(kv *v, const kv *path, int64_t s, bool write) {
    i128 k = kelem(path->u.o, 2 * s + 1).u.i;
    if (kelem(path->u.o, 2 * s).u.i) return kitemp(v, k, write);
    return kfield(v, (int64_t)k, write);
}

static kv *kpath_walk(kv *v, const kv *path, int64_t n, bool write) {
    for (int64_t s = 0; s < n; s++) v = kpath_step(v, path, s, write);
    return v;
}

static kv *kpath(kv *v, const kv *path, bool write) { return kpath_walk(v, path, kpath_len(path), write); }

/* Reads the part: an item of a packed array has no address to take. */
static kv kpath_get(kv *v, const kv *path) {
    int64_t n = kpath_len(path);
    if (n == 0) return kretain(*v);
    v = kpath_walk(v, path, n - 1, false);
    if (kelem(path->u.o, 2 * n - 2).u.i) return kitem(v, kelem(path->u.o, 2 * n - 1).u.i);
    return kretain(*kpath_step(v, path, n - 1, false));
}

static void kpath_set(kv *v, const kv *path, kv value) {
    int64_t n = kpath_len(path);
    if (n == 0) {
        kset(v, value);
        return;
    }
    v = kpath_walk(v, path, n - 1, true);
    if (kelem(path->u.o, 2 * n - 2).u.i) kitem_set(v, kelem(path->u.o, 2 * n - 1).u.i, value);
    else kset(kpath_step(v, path, n - 1, true), value);
}

static kv kpath_join(kv a, kv b) {
    int64_t n = klen(a), m = klen(b);
    kv v = karray_new(P_NONE, n + m);
    for (int64_t i = 0; i < n; i++) ITEMS(v.u.o)[i] = kelem(a.u.o, i);
    for (int64_t i = 0; i < m; i++) ITEMS(v.u.o)[n + i] = kelem(b.u.o, i);
    krelease(a);
    krelease(b);
    return v;
}

static __attribute__((noinline, cold)) void klen_fail(void) { kinternal("the length of a value that has none"); }

static inline int64_t klen_at(const kv *v) {
    if (KRARE(v->tag != T_ARRAY && v->tag != T_STR)) klen_fail();
    return klen(*v);
}

/* ---- integers ---- */

enum { K_LIT, K_I8, K_I16, K_I32, K_I64, K_I128, K_U8, K_U16, K_U32, K_U64, K_U128, K_ISIZE, K_USIZE };
static const char *const KKIND[] = {"lit", "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "isize", "usize"};
static const int KBITS[] = {128, 8, 16, 32, 64, 128, 8, 16, 32, 64, 128, 64, 64};
static const bool KSIGNED[] = {true, true, true, true, true, true, false, false, false, false, false, true, false};
#define U128_LIMIT "the interpreter only supports `u128` values below 2^127"

static void kbounds(int k, i128 *lo, i128 *hi) {
    int bits = KBITS[k];
    if (bits == 128) {
        *lo = k == K_U128 ? 0 : I128_MIN;
        *hi = I128_MAX;
    } else if (KSIGNED[k]) {
        *lo = -((i128)1 << (bits - 1));
        *hi = ((i128)1 << (bits - 1)) - 1;
    } else {
        *lo = 0;
        *hi = ((i128)1 << bits) - 1;
    }
}

enum { O_ADD, O_SUB, O_MUL, O_DIV, O_REM, O_ADDW, O_SUBW, O_MULW, O_ADDS, O_SUBS, O_MULS, O_SHL, O_SHR, O_AND, O_OR, O_XOR, O_EQ, O_NE, O_LT, O_LE, O_GT, O_GE };
static const char *const KOP[] = {"+", "-", "*", "/", "%", "+%", "-%", "*%", "+|", "-|", "*|", "<<", ">>", "&", "|", "^", "==", "!=", "<", "<=", ">", ">="};

static void koverflow(int op, int k) {
    char buf[160];
    if (k == K_U128) kpanic(U128_LIMIT);
    if (k == K_LIT) snprintf(buf, sizeof buf, "integer overflow: the result of `%s` does not fit in 128 bits", KOP[op]);
    else snprintf(buf, sizeof buf, "integer overflow: the result of `%s` does not fit in `%s`", KOP[op], KKIND[k]);
    kpanic(buf);
}

static i128 kwrap(int op, int k, i128 r) {
    int bits = KBITS[k];
    i128 lo, hi;
    kbounds(k, &lo, &hi);
    if (bits == 128) {
        if (k == K_U128 && r < 0) koverflow(op, k);
        return r;
    }
    i128 low = r & (((i128)1 << bits) - 1);
    return lo < 0 && low > hi ? low - ((i128)1 << bits) : low;
}

static i128 kint_op(int op, int k, i128 a, i128 b) {
    i128 lo, hi, r = 0;
    bool over = false;
    kbounds(k, &lo, &hi);
    switch (op) {
    case O_ADD: case O_ADDS: over = __builtin_add_overflow(a, b, &r); break;
    case O_SUB: case O_SUBS: over = __builtin_sub_overflow(a, b, &r); break;
    case O_MUL: case O_MULS: over = __builtin_mul_overflow(a, b, &r); break;
    case O_DIV: case O_REM:
        if (b == 0) kpanic("division by zero");
        if (a == I128_MIN && b == -1) over = true;
        else r = op == O_DIV ? a / b : a % b;
        break;
    case O_ADDW: return kwrap(op, k, (i128)((u128)a + (u128)b));
    case O_SUBW: return kwrap(op, k, (i128)((u128)a - (u128)b));
    case O_MULW: return kwrap(op, k, (i128)((u128)a * (u128)b));
    case O_SHL: case O_SHR: {
        int width = k == K_LIT ? 127 : KBITS[k];
        if (b < 0 || b >= width) kpanicf("a shift by ", kint(b), " is out of range");
        if (op == O_SHR) return a >> (int)b;
        r = (i128)((u128)a << (int)b);
        if (k != K_LIT) return kwrap(op, k, r);
        if (r >> (int)b != a) koverflow(op, k);
        return r;
    }
    default: kinternal("a comparison treated as arithmetic");
    }
    if (op == O_ADDS || op == O_SUBS || op == O_MULS) {
        bool up = op == O_ADDS ? b > 0 : op == O_SUBS ? b < 0 : (a < 0) == (b < 0);
        if (k == K_U128) {
            if (over) koverflow(op, k);
            return r >= 0 ? r : 0;
        }
        if (over) return up ? hi : lo;
        return r < lo ? lo : r > hi ? hi : r;
    }
    if (over || r < lo || r > hi) koverflow(op, k);
    return r;
}

/* ---- operators ---- */

static bool keq(kv a, kv b) {
    if (a.tag != b.tag) return false;
    switch (a.tag) {
    case T_UNIT: case T_NONE: return true;
    case T_INT: return a.u.i == b.u.i;
    case T_BIG: return kwide(a) == kwide(b);
    case T_F64: return a.u.f == b.u.f;
    case T_F32: return a.u.g == b.u.g;
    case T_BOOL: return a.u.t == b.u.t;
    case T_CHAR: return a.u.c == b.u.c;
    case T_FN: return a.a == b.a;
    case T_STR: return klen(a) == klen(b) && memcmp(BYTES(a.u.o), BYTES(b.u.o), (size_t)klen(a)) == 0;
    default:
        if (a.a != b.a || a.b != b.b || klen(a) != klen(b)) return false;
        for (int64_t i = 0; i < klen(a); i++) {
            kv x = kown(a.u.o, i), y = kown(b.u.o, i);
            bool same = keq(x, y);
            krelease(x);
            krelease(y);
            if (!same) return false;
        }
        return true;
    }
}

/* -1, 0 or 1; 2 when the values have no order (a NaN). */
static int kcmp(kv a, kv b) {
    if (a.tag == T_INT && b.tag == T_INT) return a.u.i < b.u.i ? -1 : a.u.i > b.u.i;
    if (KISINT(a) && KISINT(b)) return kwide(a) < kwide(b) ? -1 : kwide(a) > kwide(b);
    if (a.tag == T_F64 && b.tag == T_F64) return a.u.f < b.u.f ? -1 : a.u.f > b.u.f ? 1 : a.u.f == b.u.f ? 0 : 2;
    if (a.tag == T_F32 && b.tag == T_F32) return a.u.g < b.u.g ? -1 : a.u.g > b.u.g ? 1 : a.u.g == b.u.g ? 0 : 2;
    if (a.tag == T_CHAR && b.tag == T_CHAR) return a.u.c < b.u.c ? -1 : a.u.c > b.u.c;
    if (a.tag == T_STR && b.tag == T_STR) {
        int64_t n = klen(a) < klen(b) ? klen(a) : klen(b);
        int c = memcmp(BYTES(a.u.o), BYTES(b.u.o), (size_t)n);
        return c < 0 ? -1 : c > 0 ? 1 : klen(a) < klen(b) ? -1 : klen(a) > klen(b);
    }
    kinternal("a comparison of values that have no order");
    return 2;
}

static double kfloat_op(int op, double x, double y) {
    switch (op) {
    case O_ADD: return x + y;
    case O_SUB: return x - y;
    case O_MUL: return x * y;
    case O_DIV: return x / y;
    case O_REM: return fmod(x, y);
    default: kinternal("an integer operator applied to floats");
    }
    return 0;
}

/* `k` is an integer kind, or -1 when the values decide. Consumes both. */
static kv kbin(int op, int k, kv a, kv b) {
    kv r;
    if (op == O_EQ || op == O_NE) r = kbool(keq(a, b) == (op == O_EQ));
    else if (op >= O_LT) {
        int c = kcmp(a, b);
        r = kbool(op == O_LT ? c == -1 : op == O_LE ? c == -1 || c == 0 : op == O_GT ? c == 1 : c == 1 || c == 0);
    } else if (op >= O_AND) {
        if (KISINT(a) && KISINT(b)) r = kint(op == O_AND ? kwide(a) & kwide(b) : op == O_OR ? kwide(a) | kwide(b) : kwide(a) ^ kwide(b));
        else if (a.tag == T_BOOL && b.tag == T_BOOL) r = kbool(op == O_AND ? a.u.t & b.u.t : op == O_OR ? a.u.t | b.u.t : a.u.t ^ b.u.t);
        else kinternal("a bit operator applied to the wrong kind of value");
    } else if (KISINT(a) && KISINT(b) && k >= 0) r = kint(kint_op(op, k, kwide(a), kwide(b)));
    else if (a.tag == T_F64 && b.tag == T_F64) r = kf64(kfloat_op(op, a.u.f, b.u.f));
    else if (a.tag == T_F32 && b.tag == T_F32) r = kf32((float)kfloat_op(op, (double)a.u.g, (double)b.u.g));
    else kinternal("an arithmetic operator applied to the wrong kind of value");
    krelease(a);
    krelease(b);
    return r;
}

enum { U_NEG, U_NOT, U_BITNOT };

static kv kun(int op, int k, kv v) {
    if (op == U_NEG && KISINT(v) && k >= 0) return kint(kint_op(O_SUB, k, 0, ktakei(v)));
    if (op == U_NEG && v.tag == T_F64) return kf64(-v.u.f);
    if (op == U_NEG && v.tag == T_F32) return kf32(-v.u.g);
    if (op == U_NOT && v.tag == T_BOOL) return kbool(!v.u.t);
    if (op == U_BITNOT && KISINT(v) && k >= 0) {
        i128 lo, hi, w = ktakei(v);
        kbounds(k, &lo, &hi);
        if (lo < 0) return kint(~w);
        if (k == K_U128) kpanic(U128_LIMIT);
        return kint(hi - w);
    }
    kinternal("a unary operator applied to the wrong kind of value");
    return v;
}

static bool ktruth(kv v) {
    if (v.tag != T_BOOL) kinternal("a branch on a value that is not a bool");
    return v.u.t;
}

static kv kcast_int(kv v, int k) {
    i128 lo, hi, i = 0;
    kbounds(k, &lo, &hi);
    bool fits = true;
    switch (v.tag) {
    case T_INT: case T_BIG: i = kwide(v); break;
    case T_CHAR: i = v.u.c; break;
    case T_BOOL: return kint(v.u.t);
    case T_F64: case T_F32: {
        double t = trunc(v.tag == T_F64 ? v.u.f : (double)v.u.g);
        fits = isfinite(t) && t >= (double)lo && t <= (double)hi;
        i = t >= 170141183460469231731687303715884105728.0 ? I128_MAX : fits ? (i128)t : 0;
        break;
    }
    default: kinternal("a cast of a value that is not a number");
    }
    if (!fits || i < lo || i > hi) {
        ksb sb = {0};
        ksb_str(&sb, "`as`: the value ");
        kshow(&sb, v, false);
        ksb_str(&sb, " does not fit in `");
        ksb_str(&sb, KKIND[k]);
        ksb_str(&sb, "`");
        kpanic(sb.p);
    }
    krelease(v);
    return kint(i);
}

static kv kcast_float(kv v, bool single) {
    double f = 0;
    if (KISINT(v)) f = (double)ktakei(v);
    else if (v.tag == T_F64) f = v.u.f;
    else if (v.tag == T_F32) f = (double)v.u.g;
    else kinternal("a cast of a value that is not a number");
    return single ? kf32((float)f) : kf64(f);
}

/* Conversions are written as text by the compiler (see `conv_code`):
 *   _ nothing    Fk fit kind k    d to f64    s to f32    W c wrap in Some
 *   I c inside an optional    E c each element    Tn c.. tuple    R c c result */
static void kconv_skip(const char **s) {
    char c = *(*s)++;
    if (c == 'F') (*s)++;
    else if (c == 'W' || c == 'I' || c == 'E') kconv_skip(s);
    else if (c == 'R') kconv_skip(s), kconv_skip(s);
    else if (c == 'T')
        for (int n = *(*s)++ - 'a'; n > 0; n--) kconv_skip(s);
}

/* Applies the conversion at `*s` and steps over it. Consumes the value. */
static kv kconv_at(const char **s, kv v) {
    const char *start = *s;
    char c = *(*s)++;
    switch (c) {
    case '_': return v;
    case 'F': {
        int k = *(*s)++ - 'a';
        i128 lo, hi;
        kbounds(k, &lo, &hi);
        if (!KISINT(v)) break;
        if (kwide(v) < lo || kwide(v) > hi) {
            ksb sb = {0};
            ksb_str(&sb, "the value ");
            ksb_i128(&sb, kwide(v));
            ksb_str(&sb, " is out of range for `");
            ksb_str(&sb, KKIND[k]);
            ksb_str(&sb, "`");
            kpanic(sb.p);
        }
        return v;
    }
    case 'd': case 's':
        if (v.tag == T_F32) return v;
        return kcast_float(v, c == 's');
    case 'W': return kbox(T_SOME, kconv_at(s, v));
    case 'I':
        if (v.tag == T_NONE) {
            kconv_skip(s);
            return v;
        }
        if (v.tag != T_SOME) break;
        kunique(&v);
        ITEMS(v.u.o)[0] = kconv_at(s, ITEMS(v.u.o)[0]);
        return v;
    case 'E': {
        if (v.tag != T_ARRAY) break;
        kunique(&v);
        kunpack(&v);
        const char *each = *s;
        for (int64_t i = 0; i < klen(v); i++) {
            const char *p = each;
            ITEMS(v.u.o)[i] = kconv_at(&p, ITEMS(v.u.o)[i]);
        }
        kconv_skip(s);
        krepack(&v);
        return v;
    }
    case 'T': {
        int n = *(*s)++ - 'a';
        if (v.tag != T_TUPLE || klen(v) != n) break;
        kunique(&v);
        for (int i = 0; i < n; i++) ITEMS(v.u.o)[i] = kconv_at(s, ITEMS(v.u.o)[i]);
        return v;
    }
    case 'R': {
        if (v.tag != T_OK && v.tag != T_ERR) break;
        kunique(&v);
        const char *ok = *s;
        kconv_skip(s);
        const char *err = *s;
        kconv_skip(s);
        const char *p = v.tag == T_OK ? ok : err;
        ITEMS(v.u.o)[0] = kconv_at(&p, ITEMS(v.u.o)[0]);
        return v;
    }
    }
    (void)start;
    kinternal("a conversion applied to the wrong kind of value");
    return v;
}

static kv kconv(kv v, const char *code) { return kconv_at(&code, v); }

/* ---- the rest of the rvalues ---- */

/* Whether the value where it is holds the given variant. */
static kv kisb(const kv *v, uint32_t tag) {
    bool r = false;
    if (v->tag == T_ENUM) r = v->b == tag;
    else if (v->tag == T_NONE || v->tag == T_OK) r = tag == 0;
    else if (v->tag == T_SOME || v->tag == T_ERR) r = tag == 1;
    else kinternal("a variant test on a value without variants");
    return kbool(r);
}

static kv kis(kv v, uint32_t tag) {
    kv r = kisb(&v, tag);
    krelease(v);
    return r;
}

static kv klength(kv v) {
    if (v.tag != T_ARRAY && v.tag != T_STR) kinternal("the length of a value that has none");
    kv r = kint(klen(v));
    krelease(v);
    return r;
}

static kv krepeat(kv v, kv n) {
    if (!KISINT(n)) kinternal("an array length that is not an integer");
    if (kwide(n) < 0 || kwide(n) > 100000000) kpanicf("an array of length ", n, " cannot be created");
    int64_t len = n.u.i;
    kv r = karray_new(kpack_of(v), len);
    if (r.u.o->pack == P_BOOL) memset(r.u.o + 1, v.u.t, (size_t)len);
    else
        for (int64_t i = 0; i < len; i++) kraw(r.u.o, i, kretain(v));
    krelease(v);
    return r;
}

static kv krange_part(kv r, int which) {
    if (r.tag != T_TUPLE || klen(r) != 3) kinternal("a range with the wrong shape");
    return ITEMS(r.u.o)[which];
}

static int64_t kbound(kv part, int64_t fallback) {
    if (part.tag != T_SOME) return fallback;
    kv i = ITEMS(part.u.o)[0];
    if (!KISINT(i)) kinternal("a range bound that is not an integer");
    if (kwide(i) < 0) kpanicf("the range bound ", i, " is negative");
    return kwide(i) > INT64_MAX - 1 ? INT64_MAX - 1 : i.u.i;
}

static kv kslice(kv base, kv range) {
    int64_t len = klen(base);
    bool closed = ktruth(krange_part(range, 2)) && krange_part(range, 1).tag == T_SOME;
    int64_t lo = kbound(krange_part(range, 0), 0);
    int64_t hi = kbound(krange_part(range, 1), len) + (closed ? 1 : 0);
    char buf[160];
    if (lo > hi || hi > len) {
        snprintf(buf, sizeof buf, "the range %lld..%lld is out of range for length %lld", (long long)lo, (long long)hi, (long long)len);
        kpanic(buf);
    }
    kv r;
    if (base.tag == T_ARRAY) {
        r = karray_new(base.u.o->pack, hi - lo);
        for (int64_t i = lo; i < hi; i++) kraw(r.u.o, i - lo, kown(base.u.o, i));
    } else if (base.tag == T_STR) {
        const char *s = BYTES(base.u.o);
        if ((lo < len && (s[lo] & 0xC0) == 0x80) || (hi < len && (s[hi] & 0xC0) == 0x80)) {
            snprintf(buf, sizeof buf, "the range %lld..%lld splits a character of the string", (long long)lo, (long long)hi);
            kpanic(buf);
        }
        r = kstr(s + lo, (size_t)(hi - lo));
    } else {
        kinternal("a slice of a value that cannot be sliced");
        r = KUNIT;
    }
    krelease(base);
    krelease(range);
    return r;
}

static kv kchars(kv v) {
    if (v.tag != T_STR) kinternal("the characters of a value that is not a string");
    const unsigned char *s = (const unsigned char *)BYTES(v.u.o);
    int64_t len = klen(v), n = 0;
    for (int64_t i = 0; i < len; i++) n += (s[i] & 0xC0) != 0x80;
    kv r = karray_new(P_CHAR, n);
    int64_t out = 0;
    for (int64_t i = 0; i < len;) {
        uint32_t c = s[i];
        int k = c < 0x80 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4;
        c = k == 1 ? c : c & (0xFFu >> (k + 1));
        for (int j = 1; j < k; j++) c = (c << 6) | (s[i + j] & 0x3F);
        kraw(r.u.o, out++, kchar(c));
        i += k;
    }
    krelease(v);
    return r;
}

static kv krange_start(kv r) {
    kv lo = krange_part(r, 0);
    if (lo.tag != T_SOME) kpanic("a range without a start cannot be iterated");
    kv v = kretain(ITEMS(lo.u.o)[0]);
    krelease(r);
    return v;
}

static kv krange_has(kv r, kv i) {
    kv hi = krange_part(r, 1);
    bool has = true;
    if (hi.tag == T_SOME) {
        int c = kcmp(i, ITEMS(hi.u.o)[0]);
        has = c == -1 || (c == 0 && ktruth(krange_part(r, 2)));
    }
    krelease(r);
    krelease(i);
    return kbool(has);
}

/* The same two tests for a machine integer, leaving the range in place. */
static bool krange_has_int(const kv *r, i128 i) {
    kv hi = krange_part(*r, 1);
    if (hi.tag != T_SOME) return true;
    kv bound = ITEMS(hi.u.o)[0];
    if (!KISINT(bound)) kinternal("a range bound that is not an integer");
    return i < kwide(bound) || (i == kwide(bound) && ktruth(krange_part(*r, 2)));
}

static bool krange_last_int(const kv *r, i128 i) {
    kv hi = krange_part(*r, 1);
    if (hi.tag != T_SOME || !ktruth(krange_part(*r, 2))) return false;
    kv bound = ITEMS(hi.u.o)[0];
    return KISINT(bound) && kwide(bound) == i;
}

static kv krange_last(kv r, kv i) {
    kv hi = krange_part(r, 1);
    bool last = ktruth(krange_part(r, 2)) && hi.tag == T_SOME && keq(ITEMS(hi.u.o)[0], i);
    krelease(r);
    krelease(i);
    return kbool(last);
}

static kv ksucc(kv v, int k) {
    if (KISINT(v) && k >= 0) return kint(kint_op(O_ADD, k, ktakei(v), 1));
    if (v.tag == T_CHAR) {
        uint32_t next = v.u.c == 0xD7FF ? 0xE000 : v.u.c + 1;
        if (next > 0x10FFFF) kpanic("there is no character after the last one");
        return kchar(next);
    }
    kinternal("the successor of a value that has none");
    return v;
}

static kv kclosure(uint32_t f, int64_t n, const kv *captured) {
    kv v = kagg(T_CLOSURE, f, 0, n, captured);
    return v;
}

static kv kcapture(kv env, int64_t k) {
    if (env.tag != T_CLOSURE || k >= klen(env)) kinternal("a capture that does not exist");
    return kretain(ITEMS(env.u.o)[k]);
}

/* `panic(message)`. */
static kv kpanic_with(kv v) {
    ksb sb = {0};
    kshow(&sb, v, true);
    ksb_put(&sb, "", 1); /* the terminator */
    kpanic(sb.p ? sb.p : "");
    return v;
}

/* ---- `with io = lines` ----
 *
 * While such a block is open, what `io.print` writes is kept here instead
 * of going out, one entry for each call. The blocks nest: each remembers
 * where its lines start. It is one list for the whole program, so what a
 * task prints while the block is open is kept too. */
enum { KCAPTURE_MAX = 32 };
static struct {
    char **lines;
    size_t *lens;
    size_t n, cap;
    size_t marks[KCAPTURE_MAX];
    int depth;
    /* Whether the last entry was left without its end by `io.write`: what
       is printed next goes on in it. */
    bool open;
} kcaps[2];
#ifdef KTHREADS
static pthread_mutex_t kcap_lock = PTHREAD_MUTEX_INITIALIZER;
#define KCAP_LOCK() pthread_mutex_lock(&kcap_lock)
#define KCAP_UNLOCK() pthread_mutex_unlock(&kcap_lock)
#else
#define KCAP_LOCK() ((void)0)
#define KCAP_UNLOCK() ((void)0)
#endif

static kv kcapture_open_at(int ch) {
    KCAP_LOCK();
    if (kcaps[ch].depth == KCAPTURE_MAX) kpanic(ch ? "too many `with err` blocks, one inside another" : "too many `with io` blocks, one inside another");
    kcaps[ch].marks[kcaps[ch].depth++] = kcaps[ch].n;
    kcaps[ch].open = false;
    KCAP_UNLOCK();
    return KUNIT;
}

/* Keeps a line when a block is open, or a piece of one when `whole` is
   false. Takes the text when it does. */
static bool kcapture_at(int ch, char *text, size_t len, bool whole) {
    if (!kcaps[ch].depth) return false;
    KCAP_LOCK();
    if (!kcaps[ch].depth) {
        KCAP_UNLOCK();
        return false;
    }
    if (kcaps[ch].open) {
        size_t had = kcaps[ch].lens[kcaps[ch].n - 1];
        char *line = (char *)realloc(kcaps[ch].lines[kcaps[ch].n - 1], had + len + 1);
        if (!line) kpanic("out of memory");
        if (len) memcpy(line + had, text, len);
        kcaps[ch].lines[kcaps[ch].n - 1] = line;
        kcaps[ch].lens[kcaps[ch].n - 1] = had + len;
        kcaps[ch].open = !whole;
        KCAP_UNLOCK();
        free(text);
        return true;
    }
    kcaps[ch].open = !whole;
    if (kcaps[ch].n == kcaps[ch].cap) {
        kcaps[ch].cap = kcaps[ch].cap ? kcaps[ch].cap * 2 : 16;
        kcaps[ch].lines = (char **)realloc(kcaps[ch].lines, kcaps[ch].cap * sizeof(char *));
        kcaps[ch].lens = (size_t *)realloc(kcaps[ch].lens, kcaps[ch].cap * sizeof(size_t));
        if (!kcaps[ch].lines || !kcaps[ch].lens) kpanic("out of memory");
    }
    kcaps[ch].lines[kcaps[ch].n] = text;
    kcaps[ch].lens[kcaps[ch].n++] = len;
    KCAP_UNLOCK();
    return true;
}

/* Ends the innermost block: its lines go after the ones `old` has. */
static kv kcapture_close_at(int ch, kv old) {
    KCAP_LOCK();
    size_t from = kcaps[ch].depth ? kcaps[ch].marks[--kcaps[ch].depth] : kcaps[ch].n;
    size_t to = kcaps[ch].n;
    kcaps[ch].n = from;
    kcaps[ch].open = false;
    char **lines = NULL;
    size_t *lens = NULL;
    if (to > from) {
        /* A copy of the entries, so that the list can go on being used. */
        lines = (char **)malloc((to - from) * sizeof(char *));
        lens = (size_t *)malloc((to - from) * sizeof(size_t));
        if (!lines || !lens) kpanic("out of memory");
        memcpy(lines, kcaps[ch].lines + from, (to - from) * sizeof(char *));
        memcpy(lens, kcaps[ch].lens + from, (to - from) * sizeof(size_t));
    }
    KCAP_UNLOCK();
    for (size_t i = 0; i < to - from; i++) {
        kpush(&old, 0, NULL, kstr(lines[i] ? lines[i] : "", lens[i]));
        free(lines[i]);
    }
    free(lines);
    free(lens);
    return old;
}

static kv kcapture_open(void) { return kcapture_open_at(0); }
static kv kcapture_close(kv old) { return kcapture_close_at(0, old); }
static kv kerrors_open(void) { return kcapture_open_at(1); }
static kv kerrors_close(kv old) { return kcapture_close_at(1, old); }
static bool kcapture_line(char *text, size_t len, bool whole) { return kcapture_at(0, text, len, whole); }

static kv kprint(kv v) {
    ksb sb = {0};
    kshow(&sb, v, true);
    if (kcapture_line(sb.p, sb.len, true)) {
        krelease(v);
        return kbox(T_OK, KUNIT);
    }
    ksb_str(&sb, "\n");
    bool ok = fwrite(sb.p, 1, sb.len, stdout) == sb.len;
    free(sb.p);
    krelease(v);
    if (ok) return kbox(T_OK, KUNIT);
    return kbox(T_ERR, kstr("write error", 11));
}

/* `io.write`: the text as it is, without the end of the line. */
static kv kio_write(kv text) {
    size_t len = (size_t)klen(text);
    char *copy = (char *)malloc(len + 1);
    if (!copy) kpanic("out of memory");
    if (len) memcpy(copy, BYTES(text.u.o), len);
    krelease(text);
    if (kcapture_line(copy, len, false)) return kbox(T_OK, KUNIT);
    bool ok = fwrite(copy, 1, len, stdout) == len;
    free(copy);
    if (ok) return kbox(T_OK, KUNIT);
    return kbox(T_ERR, kstr("write error", 11));
}

/* ---- built-in modules ---- */

static int kargc;
static char **kargv;

static kv kfailed(const char *what) {
    ksb sb = {0};
    ksb_str(&sb, what);
    return kbox(T_ERR, ksb_done(&sb));
}

/* The bytes of a string. */
static const char *kbytes(kv v) { return v.u.o ? BYTES(v.u.o) : ""; }

/* The last error of the system, worded as the interpreter words it. */
static kv kos_error(void) {
    char buf[300];
    int code = errno;
    snprintf(buf, sizeof buf, "%s (os error %d)", strerror(code), code);
    return kfailed(buf);
}

static kv kio_eprint(kv v) {
    ksb sb = {0};
    kshow(&sb, v, true);
    /* Inside a `with err` block the line is kept. */
    if (kcapture_at(1, sb.p, sb.len, true)) {
        krelease(v);
        return kbox(T_OK, KUNIT);
    }
    ksb_str(&sb, "\n");
    fflush(stdout);
    bool ok = fwrite(sb.p, 1, sb.len, stderr) == sb.len;
    free(sb.p);
    krelease(v);
    return ok ? kbox(T_OK, KUNIT) : kfailed("write error");
}

/* The next line of the input without its end, or `None` when there is none. */
/* The lines `io.read_line` gives inside `with input = lines` blocks:
   the innermost one is the last. */
#define KINPUT_MAX 16
static struct {
    kv lines[KINPUT_MAX];
    int64_t next[KINPUT_MAX];
    int depth;
} kinput;

static kv kinput_open(kv lines) {
    KCAP_LOCK();
    if (kinput.depth == KINPUT_MAX) kpanic("too many `with input` blocks, one inside another");
    kinput.lines[kinput.depth] = lines;
    kinput.next[kinput.depth++] = 0;
    KCAP_UNLOCK();
    return KUNIT;
}

static kv kinput_close(void) {
    kv old = KUNIT;
    KCAP_LOCK();
    if (kinput.depth) old = kinput.lines[--kinput.depth];
    KCAP_UNLOCK();
    krelease(old);
    return KUNIT;
}

static kv kio_read_line(void) {
    KCAP_LOCK();
    if (kinput.depth) {
        int d = kinput.depth - 1;
        kv line = KNONE;
        if (kinput.next[d] < klen(kinput.lines[d])) line = kbox(T_SOME, kretain(kelem(kinput.lines[d].u.o, kinput.next[d]++)));
        KCAP_UNLOCK();
        return kbox(T_OK, line);
    }
    KCAP_UNLOCK();
    ksb sb = {0};
    int c;
    bool any = false;
    fflush(stdout);
    while ((c = getc(stdin)) != EOF) {
        any = true;
        if (c == '\n') break;
        char b = (char)c;
        ksb_put(&sb, &b, 1);
    }
    if (ferror(stdin)) {
        free(sb.p);
        return kfailed("read error");
    }
    if (!any) {
        free(sb.p);
        return kbox(T_OK, kmk(T_NONE));
    }
    if (sb.len > 0 && sb.p[sb.len - 1] == '\r') sb.len--;
    return kbox(T_OK, kbox(T_SOME, ksb_done(&sb)));
}

/* Whether some bytes are well-formed UTF-8, as every `str` must be. */
static bool kutf8(const unsigned char *s, size_t n) {
    size_t i = 0;
    while (i < n) {
        unsigned char c = s[i];
        size_t more;
        uint32_t cp, min;
        if (c < 0x80) {
            i++;
            continue;
        } else if ((c & 0xE0) == 0xC0) {
            more = 1, cp = c & 0x1F, min = 0x80;
        } else if ((c & 0xF0) == 0xE0) {
            more = 2, cp = c & 0x0F, min = 0x800;
        } else if ((c & 0xF8) == 0xF0) {
            more = 3, cp = c & 0x07, min = 0x10000;
        } else {
            return false;
        }
        if (n - i <= more) return false;
        for (size_t k = 1; k <= more; k++) {
            if ((s[i + k] & 0xC0) != 0x80) return false;
            cp = (cp << 6) | (s[i + k] & 0x3F);
        }
        if (cp < min || cp > 0x10FFFF || (cp >= 0xD800 && cp <= 0xDFFF)) return false;
        i += more + 1;
    }
    return true;
}

static kv kio_read_file(kv path) {
    FILE *f = fopen(kbytes(path), "rb");
    krelease(path);
    if (!f) return kos_error();
    ksb sb = {0};
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) ksb_put(&sb, buf, n);
    bool bad = ferror(f);
    fclose(f);
    if (bad || !kutf8((const unsigned char *)sb.p, sb.len)) {
        free(sb.p);
        return kfailed(bad ? "read error" : "stream did not contain valid UTF-8");
    }
    return kbox(T_OK, ksb_done(&sb));
}

static kv kio_write_file(kv path, kv text) {
    FILE *f = fopen(kbytes(path), "wb");
    kv r;
    if (!f) {
        r = kos_error();
    } else {
        size_t n = (size_t)klen(text);
        bool ok = fwrite(kbytes(text), 1, n, f) == n;
        ok = fclose(f) == 0 && ok;
        r = ok ? kbox(T_OK, KUNIT) : kfailed("write error");
    }
    krelease(path);
    krelease(text);
    return r;
}

/* What `io.args` gives and where `io.env` looks inside `with args = list`
   and `with env = pairs` blocks: the innermost one is the last. */
static struct {
    kv vals[KINPUT_MAX];
    int depth;
} kgiven[2];

static kv kgiven_open(int which, kv v) {
    KCAP_LOCK();
    if (kgiven[which].depth == KINPUT_MAX) kpanic(which ? "too many `with env` blocks, one inside another" : "too many `with args` blocks, one inside another");
    kgiven[which].vals[kgiven[which].depth++] = v;
    KCAP_UNLOCK();
    return KUNIT;
}

static kv kgiven_close(int which) {
    kv old = KUNIT;
    KCAP_LOCK();
    if (kgiven[which].depth) old = kgiven[which].vals[--kgiven[which].depth];
    KCAP_UNLOCK();
    krelease(old);
    return KUNIT;
}

/* The innermost value of a kind, counted once more, or unit. */
static kv kgiven_top(int which) {
    kv v = KUNIT;
    KCAP_LOCK();
    if (kgiven[which].depth) v = kretain(kgiven[which].vals[kgiven[which].depth - 1]);
    KCAP_UNLOCK();
    return v;
}

static kv kargs_open(kv list) { return kgiven_open(0, list); }
static kv kargs_close(void) { return kgiven_close(0); }
static kv kenv_open(kv pairs) { return kgiven_open(1, pairs); }
static kv kenv_close(void) { return kgiven_close(1); }

static kv kio_env(kv name) {
    if (kgiven[1].depth) {
        kv pairs = kgiven_top(1), found = KNONE;
        size_t len = (size_t)klen(name);
        for (int64_t i = klen(pairs) - 1; i >= 0; i--) {
            const kv *pair = ITEMS(kelem(pairs.u.o, i).u.o);
            if ((size_t)klen(pair[0]) == len && !memcmp(kbytes(pair[0]), kbytes(name), len)) {
                found = kbox(T_SOME, kretain(pair[1]));
                break;
            }
        }
        krelease(pairs);
        krelease(name);
        return found;
    }
    const char *v = getenv(kbytes(name));
    krelease(name);
    return v ? kbox(T_SOME, kstr(v, strlen(v))) : KNONE;
}

/* Runs a program to its end with some text as its input, and gives what
   it wrote. Its input and its error output go through files of the
   system, so that only one pipe is read here. */
static kv kio_run(kv program, kv args, kv input) {
#ifdef KPOSIX
    fflush(stdout);
    int64_t n = klen(args);
    char **argv = (char **)calloc((size_t)n + 2, sizeof(char *));
    FILE *in = tmpfile(), *err = tmpfile();
    int out[2] = {-1, -1};
    kv result;
    if (!argv || !in || !err || pipe(out) != 0) {
        result = kos_error();
        goto done;
    }
    argv[0] = (char *)kbytes(program);
    for (int64_t i = 0; i < n; i++) argv[i + 1] = (char *)kbytes(kelem(args.u.o, i));
    fwrite(kbytes(input), 1, (size_t)klen(input), in);
    fflush(in);
    rewind(in);
    posix_spawn_file_actions_t acts;
    posix_spawn_file_actions_init(&acts);
    posix_spawn_file_actions_adddup2(&acts, fileno(in), 0);
    posix_spawn_file_actions_adddup2(&acts, out[1], 1);
    posix_spawn_file_actions_adddup2(&acts, fileno(err), 2);
    posix_spawn_file_actions_addclose(&acts, out[0]);
    pid_t pid;
    int failed = posix_spawnp(&pid, argv[0], &acts, NULL, argv, environ);
    posix_spawn_file_actions_destroy(&acts);
    close(out[1]);
    out[1] = -1;
    if (failed) {
        errno = failed;
        result = kos_error();
        goto done;
    }
    ksb sb = {0};
    char buf[4096];
    ssize_t got;
    while ((got = read(out[0], buf, sizeof buf)) > 0) ksb_put(&sb, buf, (size_t)got);
    int status = 0;
    while (waitpid(pid, &status, 0) < 0 && errno == EINTR) {}
    if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
        /* The first line it wrote to the error output that says something. */
        free(sb.p);
        ksb why = {0};
        ksb_str(&why, "`");
        ksb_str(&why, argv[0]);
        ksb_str(&why, "` failed: ");
        rewind(err);
        char line[1024];
        bool said = false;
        while (!said && fgets(line, sizeof line, err)) {
            char *from = line;
            while (*from == ' ' || *from == '\t') from++;
            size_t len = strlen(from);
            while (len > 0 && (from[len - 1] == '\n' || from[len - 1] == '\r' || from[len - 1] == ' ' || from[len - 1] == '\t')) len--;
            if (len > 0) {
                ksb_put(&why, from, len);
                said = true;
            }
        }
        if (!said) {
            char num[40];
            snprintf(num, sizeof num, "status %d", WIFEXITED(status) ? WEXITSTATUS(status) : -1);
            ksb_str(&why, num);
        }
        result = kbox(T_ERR, ksb_done(&why));
    } else if (!kutf8((const unsigned char *)sb.p, sb.len)) {
        free(sb.p);
        result = kfailed("stream did not contain valid UTF-8");
    } else {
        result = kbox(T_OK, ksb_done(&sb));
    }
done:
    if (out[0] >= 0) close(out[0]);
    if (out[1] >= 0) close(out[1]);
    if (in) fclose(in);
    if (err) fclose(err);
    free(argv);
    krelease(program);
    krelease(args);
    krelease(input);
    return result;
#else
    krelease(program);
    krelease(args);
    krelease(input);
    return kfailed("running a program is not supported on this system");
#endif
}

static kv kio_args(void) {
    if (kgiven[0].depth) return kgiven_top(0);
    int n = kargc > 1 ? kargc - 1 : 0;
    kv v = karray_new(P_NONE, n);
    for (int i = 0; i < n; i++) ITEMS(v.u.o)[i] = kstr(kargv[i + 1], strlen(kargv[i + 1]));
    return v;
}

static kv kraw_concat(kv parts) {
    ksb sb = {0};
    for (int64_t i = 0; i < klen(parts); i++) {
        kv p = kelem(parts.u.o, i);
        ksb_put(&sb, kbytes(p), (size_t)klen(p));
    }
    krelease(parts);
    return ksb_done(&sb);
}

static kv kraw_from_chars(kv chars) {
    ksb sb = {0};
    for (int64_t i = 0; i < klen(chars); i++) ksb_char(&sb, kelem(chars.u.o, i).u.c);
    krelease(chars);
    return ksb_done(&sb);
}

static kv kraw_hash(kv text) {
    uint64_t h = 0xcbf29ce484222325u;
    const unsigned char *s = (const unsigned char *)kbytes(text);
    for (int64_t i = 0; i < klen(text); i++) h = (h ^ s[i]) * 0x100000001b3u;
    krelease(text);
    return kint((int64_t)h);
}

static kv kraw_mix(kv n) {
    uint64_t z = (uint64_t)ktakei(n) + 0x9e3779b97f4a7c15u;
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9u;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebu;
    return kint((int64_t)(z ^ (z >> 31)));
}

/* A string a C function gave, copied: null is no string. */
static kv kstr_c_opt(const char *s) { return s ? kbox(T_SOME, kstr(s, strlen(s))) : KNONE; }

static kv kstr_c(const char *s, const char *name) {
    if (!s) {
        ksb sb = {0};
        ksb_str(&sb, "the C function `");
        ksb_str(&sb, name);
        ksb_str(&sb, "` gave no string: declare its result as `str?`");
        kpanic(sb.p);
    }
    return kstr(s, strlen(s));
}

/* An optional boolean, integer or `f64` that the generated code keeps
   out of a block: whether it is there, and the value. */
struct KOB { bool has; bool v; };
struct KOI { bool has; int64_t v; };
struct KOF { bool has; double v; };
struct KOU { bool has; uint64_t v; };
struct KOC { bool has; uint32_t v; };
static struct KOC KOC_get(kv v) { return v.tag == T_SOME ? (struct KOC){true, ITEMS(v.u.o)[0].u.c} : (struct KOC){0}; }
static struct KOC KOC_take(kv v) { struct KOC r = KOC_get(v); krelease(v); return r; }
static kv KOC_box(struct KOC s) { return s.has ? kbox(T_SOME, kchar(s.v)) : KNONE; }
static struct KOB KOB_get(kv v) { return v.tag == T_SOME ? (struct KOB){true, ITEMS(v.u.o)[0].u.t} : (struct KOB){0}; }
static struct KOI KOI_get(kv v) { return v.tag == T_SOME ? (struct KOI){true, (int64_t)ITEMS(v.u.o)[0].u.i} : (struct KOI){0}; }
static struct KOF KOF_get(kv v) { return v.tag == T_SOME ? (struct KOF){true, ITEMS(v.u.o)[0].u.f} : (struct KOF){0}; }
static struct KOU KOU_get(kv v) { return v.tag == T_SOME ? (struct KOU){true, (uint64_t)kwide(ITEMS(v.u.o)[0])} : (struct KOU){0}; }
static struct KOB KOB_take(kv v) { struct KOB r = KOB_get(v); krelease(v); return r; }
static struct KOI KOI_take(kv v) { struct KOI r = KOI_get(v); krelease(v); return r; }
static struct KOF KOF_take(kv v) { struct KOF r = KOF_get(v); krelease(v); return r; }
static struct KOU KOU_take(kv v) { struct KOU r = KOU_get(v); krelease(v); return r; }
static kv KOB_box(struct KOB s) { return s.has ? kbox(T_SOME, kbool(s.v)) : KNONE; }
static kv KOI_box(struct KOI s) { return s.has ? kbox(T_SOME, kint((i128)s.v)) : KNONE; }
static kv KOF_box(struct KOF s) { return s.has ? kbox(T_SOME, kf64(s.v)) : KNONE; }
static kv KOU_box(struct KOU s) { return s.has ? kbox(T_SOME, kint((i128)s.v)) : KNONE; }

/* An address a C function gave or takes. It is kept as a number; null is
   no address. */
/* A number at an address of C: read, and written. */
#define KMEM(T, N) \
    static inline T kmem_##N##_load(void *p) { T v; memcpy(&v, p, sizeof v); return v; } \
    static inline void kmem_##N##_store(void *p, T v) { memcpy(p, &v, sizeof v); }
KMEM(int8_t, i8) KMEM(int16_t, i16) KMEM(int32_t, i32) KMEM(int64_t, i64)
KMEM(uint8_t, u8) KMEM(uint16_t, u16) KMEM(uint32_t, u32) KMEM(uint64_t, u64)
KMEM(float, f32) KMEM(double, f64)

static kv kptr_v_opt(const void *p) { return p ? kbox(T_SOME, kint((i128)(uintptr_t)p)) : KNONE; }

static kv kptr_v(const void *p, const char *name) {
    if (!p) {
        ksb sb = {0};
        ksb_str(&sb, "the C function `");
        ksb_str(&sb, name);
        ksb_str(&sb, "` gave a null pointer: declare its result as `ptr?`");
        kpanic(sb.p);
    }
    return kint((i128)(uintptr_t)p);
}

static void *kptr_c_opt(kv v) { return v.tag == T_NONE ? NULL : (void *)(uintptr_t)ITEMS(v.u.o)[0].u.i; }

/* `n as char`: the character with that code. */
static kv kraw_to_char(kv n) {
    i128 i = ktakei(n);
    if (i < 0 || i > 0x10FFFF || (i >= 0xD800 && i <= 0xDFFF)) {
        ksb sb = {0};
        ksb_str(&sb, "`as`: the value ");
        ksb_i128(&sb, i);
        ksb_str(&sb, " is not a character");
        kpanic(sb.p);
    }
    return kchar((uint32_t)i);
}

/* The number a text holds: a sign, digits, a fraction and an exponent,
   and nothing else. */
static kv kraw_parse_float(kv text) {
    const char *s = kbytes(text);
    int64_t n = klen(text), i = 0, start;
    bool ok = true;
    if (i < n && s[i] == '-') i++;
    start = i;
    while (i < n && s[i] >= '0' && s[i] <= '9') i++;
    ok = i > start;
    if (ok && i < n && s[i] == '.') {
        start = ++i;
        while (i < n && s[i] >= '0' && s[i] <= '9') i++;
        ok = i > start;
    }
    if (ok && i < n && (s[i] == 'e' || s[i] == 'E')) {
        i++;
        if (i < n && (s[i] == '+' || s[i] == '-')) i++;
        start = i;
        while (i < n && s[i] >= '0' && s[i] <= '9') i++;
        ok = i > start;
    }
    kv r = KNONE;
    if (ok && i == n) {
        char *copy = malloc((size_t)n + 1);
        if (!copy) kpanic("out of memory");
        memcpy(copy, s, (size_t)n);
        copy[n] = 0;
        r = kbox(T_SOME, kf64(strtod(copy, NULL)));
        free(copy);
    }
    krelease(text);
    return r;
}

/* The functions of `std.math` on numbers with a fraction, by number. */
static kv kraw_fmath(kv op, kv x, kv y) {
    double a = x.u.f, b = y.u.f, r = a;
    switch ((int)ktakei(op)) {
    case 0: r = sqrt(a); break;
    case 1: r = floor(a); break;
    case 2: r = ceil(a); break;
    case 3: r = round(a); break;
    case 4: r = trunc(a); break;
    case 5: r = sin(a); break;
    case 6: r = cos(a); break;
    case 7: r = tan(a); break;
    case 8: r = asin(a); break;
    case 9: r = acos(a); break;
    case 10: r = atan(a); break;
    case 11: r = exp(a); break;
    case 12: r = log(a); break;
    case 13: r = log10(a); break;
    case 14: r = log2(a); break;
    case 15: r = pow(a, b); break;
    case 16: r = atan2(a, b); break;
    }
    return kf64(r);
}

/* What Unicode says of the characters after ASCII: ranges with both ends,
   and for the two cases, runs of characters whose other case is at the
   same distance. Written by a program from the tables of Rust, so that the
   interpreter and the native program agree. */
static const uint32_t KU_ALPHA[][2] = {
{170,170},{181,181},{186,186},{192,214},{216,246},{248,705},{710,721},{736,740},
{748,748},{750,750},{837,837},{867,884},{886,887},{890,893},{895,895},{902,902},
{904,906},{908,908},{910,929},{931,1013},{1015,1153},{1162,1327},{1329,1366},{1369,1369},
{1376,1416},{1456,1469},{1471,1471},{1473,1474},{1476,1477},{1479,1479},{1488,1514},{1519,1522},
{1552,1562},{1568,1623},{1625,1631},{1646,1747},{1749,1756},{1761,1768},{1773,1775},{1786,1788},
{1791,1791},{1808,1855},{1869,1969},{1994,2026},{2036,2037},{2042,2042},{2048,2071},{2074,2092},
{2112,2136},{2144,2154},{2160,2183},{2185,2191},{2199,2199},{2208,2249},{2260,2271},{2275,2281},
{2288,2363},{2365,2380},{2382,2384},{2389,2403},{2417,2435},{2437,2444},{2447,2448},{2451,2472},
{2474,2480},{2482,2482},{2486,2489},{2493,2500},{2503,2504},{2507,2508},{2510,2510},{2519,2519},
{2524,2525},{2527,2531},{2544,2545},{2556,2556},{2561,2563},{2565,2570},{2575,2576},{2579,2600},
{2602,2608},{2610,2611},{2613,2614},{2616,2617},{2622,2626},{2631,2632},{2635,2636},{2641,2641},
{2649,2652},{2654,2654},{2672,2677},{2689,2691},{2693,2701},{2703,2705},{2707,2728},{2730,2736},
{2738,2739},{2741,2745},{2749,2757},{2759,2761},{2763,2764},{2768,2768},{2784,2787},{2809,2812},
{2817,2819},{2821,2828},{2831,2832},{2835,2856},{2858,2864},{2866,2867},{2869,2873},{2877,2884},
{2887,2888},{2891,2892},{2902,2903},{2908,2909},{2911,2915},{2929,2929},{2946,2947},{2949,2954},
{2958,2960},{2962,2965},{2969,2970},{2972,2972},{2974,2975},{2979,2980},{2984,2986},{2990,3001},
{3006,3010},{3014,3016},{3018,3020},{3024,3024},{3031,3031},{3072,3084},{3086,3088},{3090,3112},
{3114,3129},{3133,3140},{3142,3144},{3146,3148},{3157,3158},{3160,3162},{3164,3165},{3168,3171},
{3200,3203},{3205,3212},{3214,3216},{3218,3240},{3242,3251},{3253,3257},{3261,3268},{3270,3272},
{3274,3276},{3285,3286},{3292,3294},{3296,3299},{3313,3315},{3328,3340},{3342,3344},{3346,3386},
{3389,3396},{3398,3400},{3402,3404},{3406,3406},{3412,3415},{3423,3427},{3450,3455},{3457,3459},
{3461,3478},{3482,3505},{3507,3515},{3517,3517},{3520,3526},{3535,3540},{3542,3542},{3544,3551},
{3570,3571},{3585,3642},{3648,3654},{3661,3661},{3713,3714},{3716,3716},{3718,3722},{3724,3747},
{3749,3749},{3751,3769},{3771,3773},{3776,3780},{3782,3782},{3789,3789},{3804,3807},{3840,3840},
{3904,3911},{3913,3948},{3953,3971},{3976,3991},{3993,4028},{4096,4150},{4152,4152},{4155,4159},
{4176,4239},{4250,4253},{4256,4293},{4295,4295},{4301,4301},{4304,4346},{4348,4680},{4682,4685},
{4688,4694},{4696,4696},{4698,4701},{4704,4744},{4746,4749},{4752,4784},{4786,4789},{4792,4798},
{4800,4800},{4802,4805},{4808,4822},{4824,4880},{4882,4885},{4888,4954},{4992,5007},{5024,5109},
{5112,5117},{5121,5740},{5743,5759},{5761,5786},{5792,5866},{5870,5880},{5888,5907},{5919,5939},
{5952,5971},{5984,5996},{5998,6000},{6002,6003},{6016,6067},{6070,6088},{6103,6103},{6108,6108},
{6176,6264},{6272,6314},{6320,6389},{6400,6430},{6432,6443},{6448,6456},{6480,6509},{6512,6516},
{6528,6571},{6576,6601},{6656,6683},{6688,6750},{6753,6772},{6823,6823},{6847,6848},{6860,6862},
{6912,6963},{6965,6979},{6981,6988},{7040,7081},{7084,7087},{7098,7141},{7143,7153},{7168,7222},
{7245,7247},{7258,7293},{7296,7306},{7312,7354},{7357,7359},{7401,7404},{7406,7411},{7413,7414},
{7418,7418},{7424,7615},{7635,7668},{7680,7957},{7960,7965},{7968,8005},{8008,8013},{8016,8023},
{8025,8025},{8027,8027},{8029,8029},{8031,8061},{8064,8116},{8118,8124},{8126,8126},{8130,8132},
{8134,8140},{8144,8147},{8150,8155},{8160,8172},{8178,8180},{8182,8188},{8305,8305},{8319,8319},
{8336,8348},{8450,8450},{8455,8455},{8458,8467},{8469,8469},{8473,8477},{8484,8484},{8486,8486},
{8488,8488},{8490,8493},{8495,8505},{8508,8511},{8517,8521},{8526,8526},{8544,8584},{9398,9449},
{11264,11492},{11499,11502},{11506,11507},{11520,11557},{11559,11559},{11565,11565},{11568,11623},{11631,11631},
{11648,11670},{11680,11686},{11688,11694},{11696,11702},{11704,11710},{11712,11718},{11720,11726},{11728,11734},
{11736,11742},{11744,11775},{11823,11823},{12293,12295},{12321,12329},{12337,12341},{12344,12348},{12353,12438},
{12445,12447},{12449,12538},{12540,12543},{12549,12591},{12593,12686},{12704,12735},{12784,12799},{13312,19903},
{19968,42124},{42192,42237},{42240,42508},{42512,42527},{42538,42539},{42560,42606},{42612,42619},{42623,42735},
{42775,42783},{42786,42888},{42891,42972},{42993,43013},{43015,43047},{43072,43123},{43136,43203},{43205,43205},
{43250,43255},{43259,43259},{43261,43263},{43274,43306},{43312,43346},{43360,43388},{43392,43442},{43444,43455},
{43471,43471},{43488,43503},{43514,43518},{43520,43574},{43584,43597},{43616,43638},{43642,43710},{43712,43712},
{43714,43714},{43739,43741},{43744,43759},{43762,43765},{43777,43782},{43785,43790},{43793,43798},{43808,43814},
{43816,43822},{43824,43866},{43868,43881},{43888,44010},{44032,55203},{55216,55238},{55243,55291},{63744,64109},
{64112,64217},{64256,64262},{64275,64279},{64285,64296},{64298,64310},{64312,64316},{64318,64318},{64320,64321},
{64323,64324},{64326,64433},{64467,64829},{64848,64911},{64914,64967},{65008,65019},{65136,65140},{65142,65276},
{65313,65338},{65345,65370},{65382,65470},{65474,65479},{65482,65487},{65490,65495},{65498,65500},{65536,65547},
{65549,65574},{65576,65594},{65596,65597},{65599,65613},{65616,65629},{65664,65786},{65856,65908},{66176,66204},
{66208,66256},{66304,66335},{66349,66378},{66384,66426},{66432,66461},{66464,66499},{66504,66511},{66513,66517},
{66560,66717},{66736,66771},{66776,66811},{66816,66855},{66864,66915},{66928,66938},{66940,66954},{66956,66962},
{66964,66965},{66967,66977},{66979,66993},{66995,67001},{67003,67004},{67008,67059},{67072,67382},{67392,67413},
{67424,67431},{67456,67461},{67463,67504},{67506,67514},{67584,67589},{67592,67592},{67594,67637},{67639,67640},
{67644,67644},{67647,67669},{67680,67702},{67712,67742},{67808,67826},{67828,67829},{67840,67861},{67872,67897},
{67904,67929},{67968,68023},{68030,68031},{68096,68099},{68101,68102},{68108,68115},{68117,68119},{68121,68149},
{68192,68220},{68224,68252},{68288,68295},{68297,68324},{68352,68405},{68416,68437},{68448,68466},{68480,68497},
{68608,68680},{68736,68786},{68800,68850},{68864,68903},{68938,68965},{68969,68969},{68975,68997},{69248,69289},
{69291,69292},{69296,69297},{69314,69319},{69370,69372},{69376,69404},{69415,69415},{69424,69445},{69488,69505},
{69552,69572},{69600,69622},{69632,69701},{69745,69749},{69760,69816},{69826,69826},{69840,69864},{69888,69938},
{69956,69959},{69968,70002},{70006,70006},{70016,70079},{70081,70084},{70094,70095},{70106,70106},{70108,70108},
{70144,70161},{70163,70196},{70199,70199},{70206,70209},{70272,70278},{70280,70280},{70282,70285},{70287,70301},
{70303,70312},{70320,70376},{70400,70403},{70405,70412},{70415,70416},{70419,70440},{70442,70448},{70450,70451},
{70453,70457},{70461,70468},{70471,70472},{70475,70476},{70480,70480},{70487,70487},{70493,70499},{70528,70537},
{70539,70539},{70542,70542},{70544,70581},{70583,70592},{70594,70594},{70597,70597},{70599,70602},{70604,70605},
{70609,70609},{70611,70611},{70656,70721},{70723,70725},{70727,70730},{70751,70753},{70784,70849},{70852,70853},
{70855,70855},{71040,71093},{71096,71102},{71128,71133},{71168,71230},{71232,71232},{71236,71236},{71296,71349},
{71352,71352},{71424,71450},{71453,71466},{71488,71494},{71680,71736},{71840,71903},{71935,71942},{71945,71945},
{71948,71955},{71957,71958},{71960,71989},{71991,71992},{71995,71996},{71999,72002},{72096,72103},{72106,72151},
{72154,72159},{72161,72161},{72163,72164},{72192,72242},{72245,72254},{72272,72343},{72349,72349},{72368,72440},
{72544,72551},{72640,72672},{72704,72712},{72714,72758},{72760,72766},{72768,72768},{72818,72847},{72850,72871},
{72873,72886},{72960,72966},{72968,72969},{72971,73014},{73018,73018},{73020,73021},{73023,73025},{73027,73027},
{73030,73031},{73056,73061},{73063,73064},{73066,73102},{73104,73105},{73107,73110},{73112,73112},{73136,73179},
{73440,73462},{73472,73488},{73490,73530},{73534,73536},{73648,73648},{73728,74649},{74752,74862},{74880,75075},
{77712,77808},{77824,78895},{78913,78918},{78944,82938},{82944,83526},{90368,90414},{92160,92728},{92736,92766},
{92784,92862},{92880,92909},{92928,92975},{92992,92995},{93027,93047},{93053,93071},{93504,93548},{93760,93823},
{93856,93880},{93883,93907},{93952,94026},{94031,94087},{94095,94111},{94176,94177},{94179,94179},{94192,94198},
{94208,101589},{101631,101662},{101760,101874},{110576,110579},{110581,110587},{110589,110590},{110592,110882},{110898,110898},
{110928,110930},{110933,110933},{110948,110951},{110960,111355},{113664,113770},{113776,113788},{113792,113800},{113808,113817},
{113822,113822},{119808,119892},{119894,119964},{119966,119967},{119970,119970},{119973,119974},{119977,119980},{119982,119993},
{119995,119995},{119997,120003},{120005,120069},{120071,120074},{120077,120084},{120086,120092},{120094,120121},{120123,120126},
{120128,120132},{120134,120134},{120138,120144},{120146,120485},{120488,120512},{120514,120538},{120540,120570},{120572,120596},
{120598,120628},{120630,120654},{120656,120686},{120688,120712},{120714,120744},{120746,120770},{120772,120779},{122624,122654},
{122661,122666},{122880,122886},{122888,122904},{122907,122913},{122915,122916},{122918,122922},{122928,122989},{123023,123023},
{123136,123180},{123191,123197},{123214,123214},{123536,123565},{123584,123627},{124112,124139},{124368,124397},{124400,124400},
{124608,124638},{124640,124661},{124670,124671},{124896,124902},{124904,124907},{124909,124910},{124912,124926},{124928,125124},
{125184,125251},{125255,125255},{125259,125259},{126464,126467},{126469,126495},{126497,126498},{126500,126500},{126503,126503},
{126505,126514},{126516,126519},{126521,126521},{126523,126523},{126530,126530},{126535,126535},{126537,126537},{126539,126539},
{126541,126543},{126545,126546},{126548,126548},{126551,126551},{126553,126553},{126555,126555},{126557,126557},{126559,126559},
{126561,126562},{126564,126564},{126567,126570},{126572,126578},{126580,126583},{126585,126588},{126590,126590},{126592,126601},
{126603,126619},{126625,126627},{126629,126633},{126635,126651},{127280,127305},{127312,127337},{127344,127369},{131072,173791},
{173824,178205},{178208,183981},{183984,191456},{191472,192093},{194560,195101},{196608,201546},{201552,210041},
};
static const uint32_t KU_NUMERIC[][2] = {
{178,179},{185,185},{188,190},{1632,1641},{1776,1785},{1984,1993},{2406,2415},{2534,2543},
{2548,2553},{2662,2671},{2790,2799},{2918,2927},{2930,2935},{3046,3058},{3174,3183},{3192,3198},
{3302,3311},{3416,3422},{3430,3448},{3558,3567},{3664,3673},{3792,3801},{3872,3891},{4160,4169},
{4240,4249},{4969,4988},{5870,5872},{6112,6121},{6128,6137},{6160,6169},{6470,6479},{6608,6618},
{6784,6793},{6800,6809},{6992,7001},{7088,7097},{7232,7241},{7248,7257},{8304,8304},{8308,8313},
{8320,8329},{8528,8578},{8581,8585},{9312,9371},{9450,9471},{10102,10131},{11517,11517},{12295,12295},
{12321,12329},{12344,12346},{12690,12693},{12832,12841},{12872,12879},{12881,12895},{12928,12937},{12977,12991},
{42528,42537},{42726,42735},{43056,43061},{43216,43225},{43264,43273},{43472,43481},{43504,43513},{43600,43609},
{44016,44025},{65296,65305},{65799,65843},{65856,65912},{65930,65931},{66273,66299},{66336,66339},{66369,66369},
{66378,66378},{66513,66517},{66720,66729},{67672,67679},{67705,67711},{67751,67759},{67835,67839},{67862,67867},
{68028,68029},{68032,68047},{68050,68095},{68160,68168},{68221,68222},{68253,68255},{68331,68335},{68440,68447},
{68472,68479},{68521,68527},{68858,68863},{68912,68921},{68928,68937},{69216,69246},{69405,69414},{69457,69460},
{69573,69579},{69714,69743},{69872,69881},{69942,69951},{70096,70105},{70113,70132},{70384,70393},{70736,70745},
{70864,70873},{71248,71257},{71360,71369},{71376,71395},{71472,71483},{71904,71922},{72016,72025},{72688,72697},
{72784,72812},{73040,73049},{73120,73129},{73184,73193},{73552,73561},{73664,73684},{74752,74862},{90416,90425},
{92768,92777},{92864,92873},{93008,93017},{93019,93025},{93552,93561},{93824,93846},{94196,94198},{118000,118009},
{119488,119507},{119520,119539},{119648,119672},{120782,120831},{123200,123209},{123632,123641},{124144,124153},{124401,124410},
{125127,125135},{125264,125273},{126065,126123},{126125,126127},{126129,126132},{126209,126253},{126255,126269},{127232,127244},
{130032,130041},
};
static const uint32_t KU_SPACE[][2] = {
{133,133},{160,160},{5760,5760},{8192,8202},{8232,8233},{8239,8239},{8287,8287},{12288,12288},
};
static const uint32_t KU_UPPER[][2] = {
{192,214},{216,222},{256,256},{258,258},{260,260},{262,262},{264,264},{266,266},
{268,268},{270,270},{272,272},{274,274},{276,276},{278,278},{280,280},{282,282},
{284,284},{286,286},{288,288},{290,290},{292,292},{294,294},{296,296},{298,298},
{300,300},{302,302},{304,304},{306,306},{308,308},{310,310},{313,313},{315,315},
{317,317},{319,319},{321,321},{323,323},{325,325},{327,327},{330,330},{332,332},
{334,334},{336,336},{338,338},{340,340},{342,342},{344,344},{346,346},{348,348},
{350,350},{352,352},{354,354},{356,356},{358,358},{360,360},{362,362},{364,364},
{366,366},{368,368},{370,370},{372,372},{374,374},{376,377},{379,379},{381,381},
{385,386},{388,388},{390,391},{393,395},{398,401},{403,404},{406,408},{412,413},
{415,416},{418,418},{420,420},{422,423},{425,425},{428,428},{430,431},{433,435},
{437,437},{439,440},{444,444},{452,452},{455,455},{458,458},{461,461},{463,463},
{465,465},{467,467},{469,469},{471,471},{473,473},{475,475},{478,478},{480,480},
{482,482},{484,484},{486,486},{488,488},{490,490},{492,492},{494,494},{497,497},
{500,500},{502,504},{506,506},{508,508},{510,510},{512,512},{514,514},{516,516},
{518,518},{520,520},{522,522},{524,524},{526,526},{528,528},{530,530},{532,532},
{534,534},{536,536},{538,538},{540,540},{542,542},{544,544},{546,546},{548,548},
{550,550},{552,552},{554,554},{556,556},{558,558},{560,560},{562,562},{570,571},
{573,574},{577,577},{579,582},{584,584},{586,586},{588,588},{590,590},{880,880},
{882,882},{886,886},{895,895},{902,902},{904,906},{908,908},{910,911},{913,929},
{931,939},{975,975},{978,980},{984,984},{986,986},{988,988},{990,990},{992,992},
{994,994},{996,996},{998,998},{1000,1000},{1002,1002},{1004,1004},{1006,1006},{1012,1012},
{1015,1015},{1017,1018},{1021,1071},{1120,1120},{1122,1122},{1124,1124},{1126,1126},{1128,1128},
{1130,1130},{1132,1132},{1134,1134},{1136,1136},{1138,1138},{1140,1140},{1142,1142},{1144,1144},
{1146,1146},{1148,1148},{1150,1150},{1152,1152},{1162,1162},{1164,1164},{1166,1166},{1168,1168},
{1170,1170},{1172,1172},{1174,1174},{1176,1176},{1178,1178},{1180,1180},{1182,1182},{1184,1184},
{1186,1186},{1188,1188},{1190,1190},{1192,1192},{1194,1194},{1196,1196},{1198,1198},{1200,1200},
{1202,1202},{1204,1204},{1206,1206},{1208,1208},{1210,1210},{1212,1212},{1214,1214},{1216,1217},
{1219,1219},{1221,1221},{1223,1223},{1225,1225},{1227,1227},{1229,1229},{1232,1232},{1234,1234},
{1236,1236},{1238,1238},{1240,1240},{1242,1242},{1244,1244},{1246,1246},{1248,1248},{1250,1250},
{1252,1252},{1254,1254},{1256,1256},{1258,1258},{1260,1260},{1262,1262},{1264,1264},{1266,1266},
{1268,1268},{1270,1270},{1272,1272},{1274,1274},{1276,1276},{1278,1278},{1280,1280},{1282,1282},
{1284,1284},{1286,1286},{1288,1288},{1290,1290},{1292,1292},{1294,1294},{1296,1296},{1298,1298},
{1300,1300},{1302,1302},{1304,1304},{1306,1306},{1308,1308},{1310,1310},{1312,1312},{1314,1314},
{1316,1316},{1318,1318},{1320,1320},{1322,1322},{1324,1324},{1326,1326},{1329,1366},{4256,4293},
{4295,4295},{4301,4301},{5024,5109},{7305,7305},{7312,7354},{7357,7359},{7680,7680},{7682,7682},
{7684,7684},{7686,7686},{7688,7688},{7690,7690},{7692,7692},{7694,7694},{7696,7696},{7698,7698},
{7700,7700},{7702,7702},{7704,7704},{7706,7706},{7708,7708},{7710,7710},{7712,7712},{7714,7714},
{7716,7716},{7718,7718},{7720,7720},{7722,7722},{7724,7724},{7726,7726},{7728,7728},{7730,7730},
{7732,7732},{7734,7734},{7736,7736},{7738,7738},{7740,7740},{7742,7742},{7744,7744},{7746,7746},
{7748,7748},{7750,7750},{7752,7752},{7754,7754},{7756,7756},{7758,7758},{7760,7760},{7762,7762},
{7764,7764},{7766,7766},{7768,7768},{7770,7770},{7772,7772},{7774,7774},{7776,7776},{7778,7778},
{7780,7780},{7782,7782},{7784,7784},{7786,7786},{7788,7788},{7790,7790},{7792,7792},{7794,7794},
{7796,7796},{7798,7798},{7800,7800},{7802,7802},{7804,7804},{7806,7806},{7808,7808},{7810,7810},
{7812,7812},{7814,7814},{7816,7816},{7818,7818},{7820,7820},{7822,7822},{7824,7824},{7826,7826},
{7828,7828},{7838,7838},{7840,7840},{7842,7842},{7844,7844},{7846,7846},{7848,7848},{7850,7850},
{7852,7852},{7854,7854},{7856,7856},{7858,7858},{7860,7860},{7862,7862},{7864,7864},{7866,7866},
{7868,7868},{7870,7870},{7872,7872},{7874,7874},{7876,7876},{7878,7878},{7880,7880},{7882,7882},
{7884,7884},{7886,7886},{7888,7888},{7890,7890},{7892,7892},{7894,7894},{7896,7896},{7898,7898},
{7900,7900},{7902,7902},{7904,7904},{7906,7906},{7908,7908},{7910,7910},{7912,7912},{7914,7914},
{7916,7916},{7918,7918},{7920,7920},{7922,7922},{7924,7924},{7926,7926},{7928,7928},{7930,7930},
{7932,7932},{7934,7934},{7944,7951},{7960,7965},{7976,7983},{7992,7999},{8008,8013},{8025,8025},
{8027,8027},{8029,8029},{8031,8031},{8040,8047},{8120,8123},{8136,8139},{8152,8155},{8168,8172},
{8184,8187},{8450,8450},{8455,8455},{8459,8461},{8464,8466},{8469,8469},{8473,8477},{8484,8484},
{8486,8486},{8488,8488},{8490,8493},{8496,8499},{8510,8511},{8517,8517},{8544,8559},{8579,8579},
{9398,9423},{11264,11311},{11360,11360},{11362,11364},{11367,11367},{11369,11369},{11371,11371},{11373,11376},
{11378,11378},{11381,11381},{11390,11392},{11394,11394},{11396,11396},{11398,11398},{11400,11400},{11402,11402},
{11404,11404},{11406,11406},{11408,11408},{11410,11410},{11412,11412},{11414,11414},{11416,11416},{11418,11418},
{11420,11420},{11422,11422},{11424,11424},{11426,11426},{11428,11428},{11430,11430},{11432,11432},{11434,11434},
{11436,11436},{11438,11438},{11440,11440},{11442,11442},{11444,11444},{11446,11446},{11448,11448},{11450,11450},
{11452,11452},{11454,11454},{11456,11456},{11458,11458},{11460,11460},{11462,11462},{11464,11464},{11466,11466},
{11468,11468},{11470,11470},{11472,11472},{11474,11474},{11476,11476},{11478,11478},{11480,11480},{11482,11482},
{11484,11484},{11486,11486},{11488,11488},{11490,11490},{11499,11499},{11501,11501},{11506,11506},{42560,42560},
{42562,42562},{42564,42564},{42566,42566},{42568,42568},{42570,42570},{42572,42572},{42574,42574},{42576,42576},
{42578,42578},{42580,42580},{42582,42582},{42584,42584},{42586,42586},{42588,42588},{42590,42590},{42592,42592},
{42594,42594},{42596,42596},{42598,42598},{42600,42600},{42602,42602},{42604,42604},{42624,42624},{42626,42626},
{42628,42628},{42630,42630},{42632,42632},{42634,42634},{42636,42636},{42638,42638},{42640,42640},{42642,42642},
{42644,42644},{42646,42646},{42648,42648},{42650,42650},{42786,42786},{42788,42788},{42790,42790},{42792,42792},
{42794,42794},{42796,42796},{42798,42798},{42802,42802},{42804,42804},{42806,42806},{42808,42808},{42810,42810},
{42812,42812},{42814,42814},{42816,42816},{42818,42818},{42820,42820},{42822,42822},{42824,42824},{42826,42826},
{42828,42828},{42830,42830},{42832,42832},{42834,42834},{42836,42836},{42838,42838},{42840,42840},{42842,42842},
{42844,42844},{42846,42846},{42848,42848},{42850,42850},{42852,42852},{42854,42854},{42856,42856},{42858,42858},
{42860,42860},{42862,42862},{42873,42873},{42875,42875},{42877,42878},{42880,42880},{42882,42882},{42884,42884},
{42886,42886},{42891,42891},{42893,42893},{42896,42896},{42898,42898},{42902,42902},{42904,42904},{42906,42906},
{42908,42908},{42910,42910},{42912,42912},{42914,42914},{42916,42916},{42918,42918},{42920,42920},{42922,42926},
{42928,42932},{42934,42934},{42936,42936},{42938,42938},{42940,42940},{42942,42942},{42944,42944},{42946,42946},
{42948,42951},{42953,42953},{42955,42956},{42958,42958},{42960,42960},{42962,42962},{42964,42964},{42966,42966},
{42968,42968},{42970,42970},{42972,42972},{42997,42997},{65313,65338},{66560,66599},{66736,66771},{66928,66938},
{66940,66954},{66956,66962},{66964,66965},{68736,68786},{68944,68965},{71840,71871},{93760,93791},{93856,93880},
{119808,119833},{119860,119885},{119912,119937},{119964,119964},{119966,119967},{119970,119970},{119973,119974},{119977,119980},
{119982,119989},{120016,120041},{120068,120069},{120071,120074},{120077,120084},{120086,120092},{120120,120121},{120123,120126},
{120128,120132},{120134,120134},{120138,120144},{120172,120197},{120224,120249},{120276,120301},{120328,120353},{120380,120405},
{120432,120457},{120488,120512},{120546,120570},{120604,120628},{120662,120686},{120720,120744},{120778,120778},{125184,125217},
{127280,127305},{127312,127337},{127344,127369},
};
static const uint32_t KU_LOWER[][2] = {
{170,170},{181,181},{186,186},{223,246},{248,255},{257,257},{259,259},{261,261},
{263,263},{265,265},{267,267},{269,269},{271,271},{273,273},{275,275},{277,277},
{279,279},{281,281},{283,283},{285,285},{287,287},{289,289},{291,291},{293,293},
{295,295},{297,297},{299,299},{301,301},{303,303},{305,305},{307,307},{309,309},
{311,312},{314,314},{316,316},{318,318},{320,320},{322,322},{324,324},{326,326},
{328,329},{331,331},{333,333},{335,335},{337,337},{339,339},{341,341},{343,343},
{345,345},{347,347},{349,349},{351,351},{353,353},{355,355},{357,357},{359,359},
{361,361},{363,363},{365,365},{367,367},{369,369},{371,371},{373,373},{375,375},
{378,378},{380,380},{382,384},{387,387},{389,389},{392,392},{396,397},{402,402},
{405,405},{409,411},{414,414},{417,417},{419,419},{421,421},{424,424},{426,427},
{429,429},{432,432},{436,436},{438,438},{441,442},{445,447},{454,454},{457,457},
{460,460},{462,462},{464,464},{466,466},{468,468},{470,470},{472,472},{474,474},
{476,477},{479,479},{481,481},{483,483},{485,485},{487,487},{489,489},{491,491},
{493,493},{495,496},{499,499},{501,501},{505,505},{507,507},{509,509},{511,511},
{513,513},{515,515},{517,517},{519,519},{521,521},{523,523},{525,525},{527,527},
{529,529},{531,531},{533,533},{535,535},{537,537},{539,539},{541,541},{543,543},
{545,545},{547,547},{549,549},{551,551},{553,553},{555,555},{557,557},{559,559},
{561,561},{563,569},{572,572},{575,576},{578,578},{583,583},{585,585},{587,587},
{589,589},{591,659},{662,696},{704,705},{736,740},{837,837},{881,881},{883,883},
{887,887},{890,893},{912,912},{940,974},{976,977},{981,983},{985,985},{987,987},
{989,989},{991,991},{993,993},{995,995},{997,997},{999,999},{1001,1001},{1003,1003},
{1005,1005},{1007,1011},{1013,1013},{1016,1016},{1019,1020},{1072,1119},{1121,1121},{1123,1123},
{1125,1125},{1127,1127},{1129,1129},{1131,1131},{1133,1133},{1135,1135},{1137,1137},{1139,1139},
{1141,1141},{1143,1143},{1145,1145},{1147,1147},{1149,1149},{1151,1151},{1153,1153},{1163,1163},
{1165,1165},{1167,1167},{1169,1169},{1171,1171},{1173,1173},{1175,1175},{1177,1177},{1179,1179},
{1181,1181},{1183,1183},{1185,1185},{1187,1187},{1189,1189},{1191,1191},{1193,1193},{1195,1195},
{1197,1197},{1199,1199},{1201,1201},{1203,1203},{1205,1205},{1207,1207},{1209,1209},{1211,1211},
{1213,1213},{1215,1215},{1218,1218},{1220,1220},{1222,1222},{1224,1224},{1226,1226},{1228,1228},
{1230,1231},{1233,1233},{1235,1235},{1237,1237},{1239,1239},{1241,1241},{1243,1243},{1245,1245},
{1247,1247},{1249,1249},{1251,1251},{1253,1253},{1255,1255},{1257,1257},{1259,1259},{1261,1261},
{1263,1263},{1265,1265},{1267,1267},{1269,1269},{1271,1271},{1273,1273},{1275,1275},{1277,1277},
{1279,1279},{1281,1281},{1283,1283},{1285,1285},{1287,1287},{1289,1289},{1291,1291},{1293,1293},
{1295,1295},{1297,1297},{1299,1299},{1301,1301},{1303,1303},{1305,1305},{1307,1307},{1309,1309},
{1311,1311},{1313,1313},{1315,1315},{1317,1317},{1319,1319},{1321,1321},{1323,1323},{1325,1325},
{1327,1327},{1376,1416},{4304,4346},{4348,4351},{5112,5117},{7296,7304},{7306,7306},{7424,7615},
{7681,7681},{7683,7683},{7685,7685},{7687,7687},{7689,7689},{7691,7691},{7693,7693},{7695,7695},
{7697,7697},{7699,7699},{7701,7701},{7703,7703},{7705,7705},{7707,7707},{7709,7709},{7711,7711},
{7713,7713},{7715,7715},{7717,7717},{7719,7719},{7721,7721},{7723,7723},{7725,7725},{7727,7727},
{7729,7729},{7731,7731},{7733,7733},{7735,7735},{7737,7737},{7739,7739},{7741,7741},{7743,7743},
{7745,7745},{7747,7747},{7749,7749},{7751,7751},{7753,7753},{7755,7755},{7757,7757},{7759,7759},
{7761,7761},{7763,7763},{7765,7765},{7767,7767},{7769,7769},{7771,7771},{7773,7773},{7775,7775},
{7777,7777},{7779,7779},{7781,7781},{7783,7783},{7785,7785},{7787,7787},{7789,7789},{7791,7791},
{7793,7793},{7795,7795},{7797,7797},{7799,7799},{7801,7801},{7803,7803},{7805,7805},{7807,7807},
{7809,7809},{7811,7811},{7813,7813},{7815,7815},{7817,7817},{7819,7819},{7821,7821},{7823,7823},
{7825,7825},{7827,7827},{7829,7837},{7839,7839},{7841,7841},{7843,7843},{7845,7845},{7847,7847},
{7849,7849},{7851,7851},{7853,7853},{7855,7855},{7857,7857},{7859,7859},{7861,7861},{7863,7863},
{7865,7865},{7867,7867},{7869,7869},{7871,7871},{7873,7873},{7875,7875},{7877,7877},{7879,7879},
{7881,7881},{7883,7883},{7885,7885},{7887,7887},{7889,7889},{7891,7891},{7893,7893},{7895,7895},
{7897,7897},{7899,7899},{7901,7901},{7903,7903},{7905,7905},{7907,7907},{7909,7909},{7911,7911},
{7913,7913},{7915,7915},{7917,7917},{7919,7919},{7921,7921},{7923,7923},{7925,7925},{7927,7927},
{7929,7929},{7931,7931},{7933,7933},{7935,7943},{7952,7957},{7968,7975},{7984,7991},{8000,8005},
{8016,8023},{8032,8039},{8048,8061},{8064,8071},{8080,8087},{8096,8103},{8112,8116},{8118,8119},
{8126,8126},{8130,8132},{8134,8135},{8144,8147},{8150,8151},{8160,8167},{8178,8180},{8182,8183},
{8305,8305},{8319,8319},{8336,8348},{8458,8458},{8462,8463},{8467,8467},{8495,8495},{8500,8500},
{8505,8505},{8508,8509},{8518,8521},{8526,8526},{8560,8575},{8580,8580},{9424,9449},{11312,11359},
{11361,11361},{11365,11366},{11368,11368},{11370,11370},{11372,11372},{11377,11377},{11379,11380},{11382,11389},
{11393,11393},{11395,11395},{11397,11397},{11399,11399},{11401,11401},{11403,11403},{11405,11405},{11407,11407},
{11409,11409},{11411,11411},{11413,11413},{11415,11415},{11417,11417},{11419,11419},{11421,11421},{11423,11423},
{11425,11425},{11427,11427},{11429,11429},{11431,11431},{11433,11433},{11435,11435},{11437,11437},{11439,11439},
{11441,11441},{11443,11443},{11445,11445},{11447,11447},{11449,11449},{11451,11451},{11453,11453},{11455,11455},
{11457,11457},{11459,11459},{11461,11461},{11463,11463},{11465,11465},{11467,11467},{11469,11469},{11471,11471},
{11473,11473},{11475,11475},{11477,11477},{11479,11479},{11481,11481},{11483,11483},{11485,11485},{11487,11487},
{11489,11489},{11491,11492},{11500,11500},{11502,11502},{11507,11507},{11520,11557},{11559,11559},{11565,11565},
{42561,42561},{42563,42563},{42565,42565},{42567,42567},{42569,42569},{42571,42571},{42573,42573},{42575,42575},
{42577,42577},{42579,42579},{42581,42581},{42583,42583},{42585,42585},{42587,42587},{42589,42589},{42591,42591},
{42593,42593},{42595,42595},{42597,42597},{42599,42599},{42601,42601},{42603,42603},{42605,42605},{42625,42625},
{42627,42627},{42629,42629},{42631,42631},{42633,42633},{42635,42635},{42637,42637},{42639,42639},{42641,42641},
{42643,42643},{42645,42645},{42647,42647},{42649,42649},{42651,42653},{42787,42787},{42789,42789},{42791,42791},
{42793,42793},{42795,42795},{42797,42797},{42799,42801},{42803,42803},{42805,42805},{42807,42807},{42809,42809},
{42811,42811},{42813,42813},{42815,42815},{42817,42817},{42819,42819},{42821,42821},{42823,42823},{42825,42825},
{42827,42827},{42829,42829},{42831,42831},{42833,42833},{42835,42835},{42837,42837},{42839,42839},{42841,42841},
{42843,42843},{42845,42845},{42847,42847},{42849,42849},{42851,42851},{42853,42853},{42855,42855},{42857,42857},
{42859,42859},{42861,42861},{42863,42872},{42874,42874},{42876,42876},{42879,42879},{42881,42881},{42883,42883},
{42885,42885},{42887,42887},{42892,42892},{42894,42894},{42897,42897},{42899,42901},{42903,42903},{42905,42905},
{42907,42907},{42909,42909},{42911,42911},{42913,42913},{42915,42915},{42917,42917},{42919,42919},{42921,42921},
{42927,42927},{42933,42933},{42935,42935},{42937,42937},{42939,42939},{42941,42941},{42943,42943},{42945,42945},
{42947,42947},{42952,42952},{42954,42954},{42957,42957},{42959,42959},{42961,42961},{42963,42963},{42965,42965},
{42967,42967},{42969,42969},{42971,42971},{42993,42996},{42998,42998},{43000,43002},{43824,43866},{43868,43881},
{43888,43967},{64256,64262},{64275,64279},{65345,65370},{66600,66639},{66776,66811},{66967,66977},{66979,66993},
{66995,67001},{67003,67004},{67456,67456},{67459,67461},{67463,67504},{67506,67514},{68800,68850},{68976,68997},
{71872,71903},{93792,93823},{93883,93907},{119834,119859},{119886,119892},{119894,119911},{119938,119963},{119990,119993},
{119995,119995},{119997,120003},{120005,120015},{120042,120067},{120094,120119},{120146,120171},{120198,120223},{120250,120275},
{120302,120327},{120354,120379},{120406,120431},{120458,120485},{120514,120538},{120540,120545},{120572,120596},{120598,120603},
{120630,120654},{120656,120661},{120688,120712},{120714,120719},{120746,120770},{120772,120777},{120779,120779},{122624,122633},
{122635,122654},{122661,122666},{122928,122989},{125218,125251},
};
static const int32_t KU_TO_UPPER[][3] = {
{181,181,743},{224,246,-32},{248,254,-32},{255,255,121},{257,257,-1},{259,259,-1},
{261,261,-1},{263,263,-1},{265,265,-1},{267,267,-1},{269,269,-1},{271,271,-1},
{273,273,-1},{275,275,-1},{277,277,-1},{279,279,-1},{281,281,-1},{283,283,-1},
{285,285,-1},{287,287,-1},{289,289,-1},{291,291,-1},{293,293,-1},{295,295,-1},
{297,297,-1},{299,299,-1},{301,301,-1},{303,303,-1},{305,305,-232},{307,307,-1},
{309,309,-1},{311,311,-1},{314,314,-1},{316,316,-1},{318,318,-1},{320,320,-1},
{322,322,-1},{324,324,-1},{326,326,-1},{328,328,-1},{331,331,-1},{333,333,-1},
{335,335,-1},{337,337,-1},{339,339,-1},{341,341,-1},{343,343,-1},{345,345,-1},
{347,347,-1},{349,349,-1},{351,351,-1},{353,353,-1},{355,355,-1},{357,357,-1},
{359,359,-1},{361,361,-1},{363,363,-1},{365,365,-1},{367,367,-1},{369,369,-1},
{371,371,-1},{373,373,-1},{375,375,-1},{378,378,-1},{380,380,-1},{382,382,-1},
{383,383,-300},{384,384,195},{387,387,-1},{389,389,-1},{392,392,-1},{396,396,-1},
{402,402,-1},{405,405,97},{409,409,-1},{410,410,163},{411,411,42561},{414,414,130},
{417,417,-1},{419,419,-1},{421,421,-1},{424,424,-1},{429,429,-1},{432,432,-1},
{436,436,-1},{438,438,-1},{441,441,-1},{445,445,-1},{447,447,56},{453,453,-1},
{454,454,-2},{456,456,-1},{457,457,-2},{459,459,-1},{460,460,-2},{462,462,-1},
{464,464,-1},{466,466,-1},{468,468,-1},{470,470,-1},{472,472,-1},{474,474,-1},
{476,476,-1},{477,477,-79},{479,479,-1},{481,481,-1},{483,483,-1},{485,485,-1},
{487,487,-1},{489,489,-1},{491,491,-1},{493,493,-1},{495,495,-1},{498,498,-1},
{499,499,-2},{501,501,-1},{505,505,-1},{507,507,-1},{509,509,-1},{511,511,-1},
{513,513,-1},{515,515,-1},{517,517,-1},{519,519,-1},{521,521,-1},{523,523,-1},
{525,525,-1},{527,527,-1},{529,529,-1},{531,531,-1},{533,533,-1},{535,535,-1},
{537,537,-1},{539,539,-1},{541,541,-1},{543,543,-1},{547,547,-1},{549,549,-1},
{551,551,-1},{553,553,-1},{555,555,-1},{557,557,-1},{559,559,-1},{561,561,-1},
{563,563,-1},{572,572,-1},{575,576,10815},{578,578,-1},{583,583,-1},{585,585,-1},
{587,587,-1},{589,589,-1},{591,591,-1},{592,592,10783},{593,593,10780},{594,594,10782},
{595,595,-210},{596,596,-206},{598,599,-205},{601,601,-202},{603,603,-203},{604,604,42319},
{608,608,-205},{609,609,42315},{611,611,-207},{612,612,42343},{613,613,42280},{614,614,42308},
{616,616,-209},{617,617,-211},{618,618,42308},{619,619,10743},{620,620,42305},{623,623,-211},
{625,625,10749},{626,626,-213},{629,629,-214},{637,637,10727},{640,640,-218},{642,642,42307},
{643,643,-218},{647,647,42282},{648,648,-218},{649,649,-69},{650,651,-217},{652,652,-71},
{658,658,-219},{669,669,42261},{670,670,42258},{837,837,84},{881,881,-1},{883,883,-1},
{887,887,-1},{891,893,130},{940,940,-38},{941,943,-37},{945,961,-32},{962,962,-31},
{963,971,-32},{972,972,-64},{973,974,-63},{976,976,-62},{977,977,-57},{981,981,-47},
{982,982,-54},{983,983,-8},{985,985,-1},{987,987,-1},{989,989,-1},{991,991,-1},
{993,993,-1},{995,995,-1},{997,997,-1},{999,999,-1},{1001,1001,-1},{1003,1003,-1},
{1005,1005,-1},{1007,1007,-1},{1008,1008,-86},{1009,1009,-80},{1010,1010,7},{1011,1011,-116},
{1013,1013,-96},{1016,1016,-1},{1019,1019,-1},{1072,1103,-32},{1104,1119,-80},{1121,1121,-1},
{1123,1123,-1},{1125,1125,-1},{1127,1127,-1},{1129,1129,-1},{1131,1131,-1},{1133,1133,-1},
{1135,1135,-1},{1137,1137,-1},{1139,1139,-1},{1141,1141,-1},{1143,1143,-1},{1145,1145,-1},
{1147,1147,-1},{1149,1149,-1},{1151,1151,-1},{1153,1153,-1},{1163,1163,-1},{1165,1165,-1},
{1167,1167,-1},{1169,1169,-1},{1171,1171,-1},{1173,1173,-1},{1175,1175,-1},{1177,1177,-1},
{1179,1179,-1},{1181,1181,-1},{1183,1183,-1},{1185,1185,-1},{1187,1187,-1},{1189,1189,-1},
{1191,1191,-1},{1193,1193,-1},{1195,1195,-1},{1197,1197,-1},{1199,1199,-1},{1201,1201,-1},
{1203,1203,-1},{1205,1205,-1},{1207,1207,-1},{1209,1209,-1},{1211,1211,-1},{1213,1213,-1},
{1215,1215,-1},{1218,1218,-1},{1220,1220,-1},{1222,1222,-1},{1224,1224,-1},{1226,1226,-1},
{1228,1228,-1},{1230,1230,-1},{1231,1231,-15},{1233,1233,-1},{1235,1235,-1},{1237,1237,-1},
{1239,1239,-1},{1241,1241,-1},{1243,1243,-1},{1245,1245,-1},{1247,1247,-1},{1249,1249,-1},
{1251,1251,-1},{1253,1253,-1},{1255,1255,-1},{1257,1257,-1},{1259,1259,-1},{1261,1261,-1},
{1263,1263,-1},{1265,1265,-1},{1267,1267,-1},{1269,1269,-1},{1271,1271,-1},{1273,1273,-1},
{1275,1275,-1},{1277,1277,-1},{1279,1279,-1},{1281,1281,-1},{1283,1283,-1},{1285,1285,-1},
{1287,1287,-1},{1289,1289,-1},{1291,1291,-1},{1293,1293,-1},{1295,1295,-1},{1297,1297,-1},
{1299,1299,-1},{1301,1301,-1},{1303,1303,-1},{1305,1305,-1},{1307,1307,-1},{1309,1309,-1},
{1311,1311,-1},{1313,1313,-1},{1315,1315,-1},{1317,1317,-1},{1319,1319,-1},{1321,1321,-1},
{1323,1323,-1},{1325,1325,-1},{1327,1327,-1},{1377,1414,-48},{4304,4346,3008},{4349,4351,3008},
{5112,5117,-8},{7296,7296,-6254},{7297,7297,-6253},{7298,7298,-6244},{7299,7300,-6242},{7301,7301,-6243},
{7302,7302,-6236},{7303,7303,-6181},{7304,7304,35266},{7306,7306,-1},{7545,7545,35332},{7549,7549,3814},
{7566,7566,35384},{7681,7681,-1},{7683,7683,-1},{7685,7685,-1},{7687,7687,-1},{7689,7689,-1},
{7691,7691,-1},{7693,7693,-1},{7695,7695,-1},{7697,7697,-1},{7699,7699,-1},{7701,7701,-1},
{7703,7703,-1},{7705,7705,-1},{7707,7707,-1},{7709,7709,-1},{7711,7711,-1},{7713,7713,-1},
{7715,7715,-1},{7717,7717,-1},{7719,7719,-1},{7721,7721,-1},{7723,7723,-1},{7725,7725,-1},
{7727,7727,-1},{7729,7729,-1},{7731,7731,-1},{7733,7733,-1},{7735,7735,-1},{7737,7737,-1},
{7739,7739,-1},{7741,7741,-1},{7743,7743,-1},{7745,7745,-1},{7747,7747,-1},{7749,7749,-1},
{7751,7751,-1},{7753,7753,-1},{7755,7755,-1},{7757,7757,-1},{7759,7759,-1},{7761,7761,-1},
{7763,7763,-1},{7765,7765,-1},{7767,7767,-1},{7769,7769,-1},{7771,7771,-1},{7773,7773,-1},
{7775,7775,-1},{7777,7777,-1},{7779,7779,-1},{7781,7781,-1},{7783,7783,-1},{7785,7785,-1},
{7787,7787,-1},{7789,7789,-1},{7791,7791,-1},{7793,7793,-1},{7795,7795,-1},{7797,7797,-1},
{7799,7799,-1},{7801,7801,-1},{7803,7803,-1},{7805,7805,-1},{7807,7807,-1},{7809,7809,-1},
{7811,7811,-1},{7813,7813,-1},{7815,7815,-1},{7817,7817,-1},{7819,7819,-1},{7821,7821,-1},
{7823,7823,-1},{7825,7825,-1},{7827,7827,-1},{7829,7829,-1},{7835,7835,-59},{7841,7841,-1},
{7843,7843,-1},{7845,7845,-1},{7847,7847,-1},{7849,7849,-1},{7851,7851,-1},{7853,7853,-1},
{7855,7855,-1},{7857,7857,-1},{7859,7859,-1},{7861,7861,-1},{7863,7863,-1},{7865,7865,-1},
{7867,7867,-1},{7869,7869,-1},{7871,7871,-1},{7873,7873,-1},{7875,7875,-1},{7877,7877,-1},
{7879,7879,-1},{7881,7881,-1},{7883,7883,-1},{7885,7885,-1},{7887,7887,-1},{7889,7889,-1},
{7891,7891,-1},{7893,7893,-1},{7895,7895,-1},{7897,7897,-1},{7899,7899,-1},{7901,7901,-1},
{7903,7903,-1},{7905,7905,-1},{7907,7907,-1},{7909,7909,-1},{7911,7911,-1},{7913,7913,-1},
{7915,7915,-1},{7917,7917,-1},{7919,7919,-1},{7921,7921,-1},{7923,7923,-1},{7925,7925,-1},
{7927,7927,-1},{7929,7929,-1},{7931,7931,-1},{7933,7933,-1},{7935,7935,-1},{7936,7943,8},
{7952,7957,8},{7968,7975,8},{7984,7991,8},{8000,8005,8},{8017,8017,8},{8019,8019,8},
{8021,8021,8},{8023,8023,8},{8032,8039,8},{8048,8049,74},{8050,8053,86},{8054,8055,100},
{8056,8057,128},{8058,8059,112},{8060,8061,126},{8112,8113,8},{8126,8126,-7205},{8144,8145,8},
{8160,8161,8},{8165,8165,7},{8526,8526,-28},{8560,8575,-16},{8580,8580,-1},{9424,9449,-26},
{11312,11359,-48},{11361,11361,-1},{11365,11365,-10795},{11366,11366,-10792},{11368,11368,-1},{11370,11370,-1},
{11372,11372,-1},{11379,11379,-1},{11382,11382,-1},{11393,11393,-1},{11395,11395,-1},{11397,11397,-1},
{11399,11399,-1},{11401,11401,-1},{11403,11403,-1},{11405,11405,-1},{11407,11407,-1},{11409,11409,-1},
{11411,11411,-1},{11413,11413,-1},{11415,11415,-1},{11417,11417,-1},{11419,11419,-1},{11421,11421,-1},
{11423,11423,-1},{11425,11425,-1},{11427,11427,-1},{11429,11429,-1},{11431,11431,-1},{11433,11433,-1},
{11435,11435,-1},{11437,11437,-1},{11439,11439,-1},{11441,11441,-1},{11443,11443,-1},{11445,11445,-1},
{11447,11447,-1},{11449,11449,-1},{11451,11451,-1},{11453,11453,-1},{11455,11455,-1},{11457,11457,-1},
{11459,11459,-1},{11461,11461,-1},{11463,11463,-1},{11465,11465,-1},{11467,11467,-1},{11469,11469,-1},
{11471,11471,-1},{11473,11473,-1},{11475,11475,-1},{11477,11477,-1},{11479,11479,-1},{11481,11481,-1},
{11483,11483,-1},{11485,11485,-1},{11487,11487,-1},{11489,11489,-1},{11491,11491,-1},{11500,11500,-1},
{11502,11502,-1},{11507,11507,-1},{11520,11557,-7264},{11559,11559,-7264},{11565,11565,-7264},{42561,42561,-1},
{42563,42563,-1},{42565,42565,-1},{42567,42567,-1},{42569,42569,-1},{42571,42571,-1},{42573,42573,-1},
{42575,42575,-1},{42577,42577,-1},{42579,42579,-1},{42581,42581,-1},{42583,42583,-1},{42585,42585,-1},
{42587,42587,-1},{42589,42589,-1},{42591,42591,-1},{42593,42593,-1},{42595,42595,-1},{42597,42597,-1},
{42599,42599,-1},{42601,42601,-1},{42603,42603,-1},{42605,42605,-1},{42625,42625,-1},{42627,42627,-1},
{42629,42629,-1},{42631,42631,-1},{42633,42633,-1},{42635,42635,-1},{42637,42637,-1},{42639,42639,-1},
{42641,42641,-1},{42643,42643,-1},{42645,42645,-1},{42647,42647,-1},{42649,42649,-1},{42651,42651,-1},
{42787,42787,-1},{42789,42789,-1},{42791,42791,-1},{42793,42793,-1},{42795,42795,-1},{42797,42797,-1},
{42799,42799,-1},{42803,42803,-1},{42805,42805,-1},{42807,42807,-1},{42809,42809,-1},{42811,42811,-1},
{42813,42813,-1},{42815,42815,-1},{42817,42817,-1},{42819,42819,-1},{42821,42821,-1},{42823,42823,-1},
{42825,42825,-1},{42827,42827,-1},{42829,42829,-1},{42831,42831,-1},{42833,42833,-1},{42835,42835,-1},
{42837,42837,-1},{42839,42839,-1},{42841,42841,-1},{42843,42843,-1},{42845,42845,-1},{42847,42847,-1},
{42849,42849,-1},{42851,42851,-1},{42853,42853,-1},{42855,42855,-1},{42857,42857,-1},{42859,42859,-1},
{42861,42861,-1},{42863,42863,-1},{42874,42874,-1},{42876,42876,-1},{42879,42879,-1},{42881,42881,-1},
{42883,42883,-1},{42885,42885,-1},{42887,42887,-1},{42892,42892,-1},{42897,42897,-1},{42899,42899,-1},
{42900,42900,48},{42903,42903,-1},{42905,42905,-1},{42907,42907,-1},{42909,42909,-1},{42911,42911,-1},
{42913,42913,-1},{42915,42915,-1},{42917,42917,-1},{42919,42919,-1},{42921,42921,-1},{42933,42933,-1},
{42935,42935,-1},{42937,42937,-1},{42939,42939,-1},{42941,42941,-1},{42943,42943,-1},{42945,42945,-1},
{42947,42947,-1},{42952,42952,-1},{42954,42954,-1},{42957,42957,-1},{42959,42959,-1},{42961,42961,-1},
{42963,42963,-1},{42965,42965,-1},{42967,42967,-1},{42969,42969,-1},{42971,42971,-1},{42998,42998,-1},
{43859,43859,-928},{43888,43967,-38864},{65345,65370,-32},{66600,66639,-40},{66776,66811,-40},{66967,66977,-39},
{66979,66993,-39},{66995,67001,-39},{67003,67004,-39},{68800,68850,-64},{68976,68997,-32},{71872,71903,-32},
{93792,93823,-32},{93883,93907,-27},{125218,125251,-34},
};
static const int32_t KU_TO_LOWER[][3] = {
{192,214,32},{216,222,32},{256,256,1},{258,258,1},{260,260,1},{262,262,1},
{264,264,1},{266,266,1},{268,268,1},{270,270,1},{272,272,1},{274,274,1},
{276,276,1},{278,278,1},{280,280,1},{282,282,1},{284,284,1},{286,286,1},
{288,288,1},{290,290,1},{292,292,1},{294,294,1},{296,296,1},{298,298,1},
{300,300,1},{302,302,1},{306,306,1},{308,308,1},{310,310,1},{313,313,1},
{315,315,1},{317,317,1},{319,319,1},{321,321,1},{323,323,1},{325,325,1},
{327,327,1},{330,330,1},{332,332,1},{334,334,1},{336,336,1},{338,338,1},
{340,340,1},{342,342,1},{344,344,1},{346,346,1},{348,348,1},{350,350,1},
{352,352,1},{354,354,1},{356,356,1},{358,358,1},{360,360,1},{362,362,1},
{364,364,1},{366,366,1},{368,368,1},{370,370,1},{372,372,1},{374,374,1},
{376,376,-121},{377,377,1},{379,379,1},{381,381,1},{385,385,210},{386,386,1},
{388,388,1},{390,390,206},{391,391,1},{393,394,205},{395,395,1},{398,398,79},
{399,399,202},{400,400,203},{401,401,1},{403,403,205},{404,404,207},{406,406,211},
{407,407,209},{408,408,1},{412,412,211},{413,413,213},{415,415,214},{416,416,1},
{418,418,1},{420,420,1},{422,422,218},{423,423,1},{425,425,218},{428,428,1},
{430,430,218},{431,431,1},{433,434,217},{435,435,1},{437,437,1},{439,439,219},
{440,440,1},{444,444,1},{452,452,2},{453,453,1},{455,455,2},{456,456,1},
{458,458,2},{459,459,1},{461,461,1},{463,463,1},{465,465,1},{467,467,1},
{469,469,1},{471,471,1},{473,473,1},{475,475,1},{478,478,1},{480,480,1},
{482,482,1},{484,484,1},{486,486,1},{488,488,1},{490,490,1},{492,492,1},
{494,494,1},{497,497,2},{498,498,1},{500,500,1},{502,502,-97},{503,503,-56},
{504,504,1},{506,506,1},{508,508,1},{510,510,1},{512,512,1},{514,514,1},
{516,516,1},{518,518,1},{520,520,1},{522,522,1},{524,524,1},{526,526,1},
{528,528,1},{530,530,1},{532,532,1},{534,534,1},{536,536,1},{538,538,1},
{540,540,1},{542,542,1},{544,544,-130},{546,546,1},{548,548,1},{550,550,1},
{552,552,1},{554,554,1},{556,556,1},{558,558,1},{560,560,1},{562,562,1},
{570,570,10795},{571,571,1},{573,573,-163},{574,574,10792},{577,577,1},{579,579,-195},
{580,580,69},{581,581,71},{582,582,1},{584,584,1},{586,586,1},{588,588,1},
{590,590,1},{880,880,1},{882,882,1},{886,886,1},{895,895,116},{902,902,38},
{904,906,37},{908,908,64},{910,911,63},{913,929,32},{931,939,32},{975,975,8},
{984,984,1},{986,986,1},{988,988,1},{990,990,1},{992,992,1},{994,994,1},
{996,996,1},{998,998,1},{1000,1000,1},{1002,1002,1},{1004,1004,1},{1006,1006,1},
{1012,1012,-60},{1015,1015,1},{1017,1017,-7},{1018,1018,1},{1021,1023,-130},{1024,1039,80},
{1040,1071,32},{1120,1120,1},{1122,1122,1},{1124,1124,1},{1126,1126,1},{1128,1128,1},
{1130,1130,1},{1132,1132,1},{1134,1134,1},{1136,1136,1},{1138,1138,1},{1140,1140,1},
{1142,1142,1},{1144,1144,1},{1146,1146,1},{1148,1148,1},{1150,1150,1},{1152,1152,1},
{1162,1162,1},{1164,1164,1},{1166,1166,1},{1168,1168,1},{1170,1170,1},{1172,1172,1},
{1174,1174,1},{1176,1176,1},{1178,1178,1},{1180,1180,1},{1182,1182,1},{1184,1184,1},
{1186,1186,1},{1188,1188,1},{1190,1190,1},{1192,1192,1},{1194,1194,1},{1196,1196,1},
{1198,1198,1},{1200,1200,1},{1202,1202,1},{1204,1204,1},{1206,1206,1},{1208,1208,1},
{1210,1210,1},{1212,1212,1},{1214,1214,1},{1216,1216,15},{1217,1217,1},{1219,1219,1},
{1221,1221,1},{1223,1223,1},{1225,1225,1},{1227,1227,1},{1229,1229,1},{1232,1232,1},
{1234,1234,1},{1236,1236,1},{1238,1238,1},{1240,1240,1},{1242,1242,1},{1244,1244,1},
{1246,1246,1},{1248,1248,1},{1250,1250,1},{1252,1252,1},{1254,1254,1},{1256,1256,1},
{1258,1258,1},{1260,1260,1},{1262,1262,1},{1264,1264,1},{1266,1266,1},{1268,1268,1},
{1270,1270,1},{1272,1272,1},{1274,1274,1},{1276,1276,1},{1278,1278,1},{1280,1280,1},
{1282,1282,1},{1284,1284,1},{1286,1286,1},{1288,1288,1},{1290,1290,1},{1292,1292,1},
{1294,1294,1},{1296,1296,1},{1298,1298,1},{1300,1300,1},{1302,1302,1},{1304,1304,1},
{1306,1306,1},{1308,1308,1},{1310,1310,1},{1312,1312,1},{1314,1314,1},{1316,1316,1},
{1318,1318,1},{1320,1320,1},{1322,1322,1},{1324,1324,1},{1326,1326,1},{1329,1366,48},
{4256,4293,7264},{4295,4295,7264},{4301,4301,7264},{5024,5103,38864},{5104,5109,8},{7305,7305,1},
{7312,7354,-3008},{7357,7359,-3008},{7680,7680,1},{7682,7682,1},{7684,7684,1},{7686,7686,1},
{7688,7688,1},{7690,7690,1},{7692,7692,1},{7694,7694,1},{7696,7696,1},{7698,7698,1},
{7700,7700,1},{7702,7702,1},{7704,7704,1},{7706,7706,1},{7708,7708,1},{7710,7710,1},
{7712,7712,1},{7714,7714,1},{7716,7716,1},{7718,7718,1},{7720,7720,1},{7722,7722,1},
{7724,7724,1},{7726,7726,1},{7728,7728,1},{7730,7730,1},{7732,7732,1},{7734,7734,1},
{7736,7736,1},{7738,7738,1},{7740,7740,1},{7742,7742,1},{7744,7744,1},{7746,7746,1},
{7748,7748,1},{7750,7750,1},{7752,7752,1},{7754,7754,1},{7756,7756,1},{7758,7758,1},
{7760,7760,1},{7762,7762,1},{7764,7764,1},{7766,7766,1},{7768,7768,1},{7770,7770,1},
{7772,7772,1},{7774,7774,1},{7776,7776,1},{7778,7778,1},{7780,7780,1},{7782,7782,1},
{7784,7784,1},{7786,7786,1},{7788,7788,1},{7790,7790,1},{7792,7792,1},{7794,7794,1},
{7796,7796,1},{7798,7798,1},{7800,7800,1},{7802,7802,1},{7804,7804,1},{7806,7806,1},
{7808,7808,1},{7810,7810,1},{7812,7812,1},{7814,7814,1},{7816,7816,1},{7818,7818,1},
{7820,7820,1},{7822,7822,1},{7824,7824,1},{7826,7826,1},{7828,7828,1},{7838,7838,-7615},
{7840,7840,1},{7842,7842,1},{7844,7844,1},{7846,7846,1},{7848,7848,1},{7850,7850,1},
{7852,7852,1},{7854,7854,1},{7856,7856,1},{7858,7858,1},{7860,7860,1},{7862,7862,1},
{7864,7864,1},{7866,7866,1},{7868,7868,1},{7870,7870,1},{7872,7872,1},{7874,7874,1},
{7876,7876,1},{7878,7878,1},{7880,7880,1},{7882,7882,1},{7884,7884,1},{7886,7886,1},
{7888,7888,1},{7890,7890,1},{7892,7892,1},{7894,7894,1},{7896,7896,1},{7898,7898,1},
{7900,7900,1},{7902,7902,1},{7904,7904,1},{7906,7906,1},{7908,7908,1},{7910,7910,1},
{7912,7912,1},{7914,7914,1},{7916,7916,1},{7918,7918,1},{7920,7920,1},{7922,7922,1},
{7924,7924,1},{7926,7926,1},{7928,7928,1},{7930,7930,1},{7932,7932,1},{7934,7934,1},
{7944,7951,-8},{7960,7965,-8},{7976,7983,-8},{7992,7999,-8},{8008,8013,-8},{8025,8025,-8},
{8027,8027,-8},{8029,8029,-8},{8031,8031,-8},{8040,8047,-8},{8072,8079,-8},{8088,8095,-8},
{8104,8111,-8},{8120,8121,-8},{8122,8123,-74},{8124,8124,-9},{8136,8139,-86},{8140,8140,-9},
{8152,8153,-8},{8154,8155,-100},{8168,8169,-8},{8170,8171,-112},{8172,8172,-7},{8184,8185,-128},
{8186,8187,-126},{8188,8188,-9},{8486,8486,-7517},{8490,8490,-8383},{8491,8491,-8262},{8498,8498,28},
{8544,8559,16},{8579,8579,1},{9398,9423,26},{11264,11311,48},{11360,11360,1},{11362,11362,-10743},
{11363,11363,-3814},{11364,11364,-10727},{11367,11367,1},{11369,11369,1},{11371,11371,1},{11373,11373,-10780},
{11374,11374,-10749},{11375,11375,-10783},{11376,11376,-10782},{11378,11378,1},{11381,11381,1},{11390,11391,-10815},
{11392,11392,1},{11394,11394,1},{11396,11396,1},{11398,11398,1},{11400,11400,1},{11402,11402,1},
{11404,11404,1},{11406,11406,1},{11408,11408,1},{11410,11410,1},{11412,11412,1},{11414,11414,1},
{11416,11416,1},{11418,11418,1},{11420,11420,1},{11422,11422,1},{11424,11424,1},{11426,11426,1},
{11428,11428,1},{11430,11430,1},{11432,11432,1},{11434,11434,1},{11436,11436,1},{11438,11438,1},
{11440,11440,1},{11442,11442,1},{11444,11444,1},{11446,11446,1},{11448,11448,1},{11450,11450,1},
{11452,11452,1},{11454,11454,1},{11456,11456,1},{11458,11458,1},{11460,11460,1},{11462,11462,1},
{11464,11464,1},{11466,11466,1},{11468,11468,1},{11470,11470,1},{11472,11472,1},{11474,11474,1},
{11476,11476,1},{11478,11478,1},{11480,11480,1},{11482,11482,1},{11484,11484,1},{11486,11486,1},
{11488,11488,1},{11490,11490,1},{11499,11499,1},{11501,11501,1},{11506,11506,1},{42560,42560,1},
{42562,42562,1},{42564,42564,1},{42566,42566,1},{42568,42568,1},{42570,42570,1},{42572,42572,1},
{42574,42574,1},{42576,42576,1},{42578,42578,1},{42580,42580,1},{42582,42582,1},{42584,42584,1},
{42586,42586,1},{42588,42588,1},{42590,42590,1},{42592,42592,1},{42594,42594,1},{42596,42596,1},
{42598,42598,1},{42600,42600,1},{42602,42602,1},{42604,42604,1},{42624,42624,1},{42626,42626,1},
{42628,42628,1},{42630,42630,1},{42632,42632,1},{42634,42634,1},{42636,42636,1},{42638,42638,1},
{42640,42640,1},{42642,42642,1},{42644,42644,1},{42646,42646,1},{42648,42648,1},{42650,42650,1},
{42786,42786,1},{42788,42788,1},{42790,42790,1},{42792,42792,1},{42794,42794,1},{42796,42796,1},
{42798,42798,1},{42802,42802,1},{42804,42804,1},{42806,42806,1},{42808,42808,1},{42810,42810,1},
{42812,42812,1},{42814,42814,1},{42816,42816,1},{42818,42818,1},{42820,42820,1},{42822,42822,1},
{42824,42824,1},{42826,42826,1},{42828,42828,1},{42830,42830,1},{42832,42832,1},{42834,42834,1},
{42836,42836,1},{42838,42838,1},{42840,42840,1},{42842,42842,1},{42844,42844,1},{42846,42846,1},
{42848,42848,1},{42850,42850,1},{42852,42852,1},{42854,42854,1},{42856,42856,1},{42858,42858,1},
{42860,42860,1},{42862,42862,1},{42873,42873,1},{42875,42875,1},{42877,42877,-35332},{42878,42878,1},
{42880,42880,1},{42882,42882,1},{42884,42884,1},{42886,42886,1},{42891,42891,1},{42893,42893,-42280},
{42896,42896,1},{42898,42898,1},{42902,42902,1},{42904,42904,1},{42906,42906,1},{42908,42908,1},
{42910,42910,1},{42912,42912,1},{42914,42914,1},{42916,42916,1},{42918,42918,1},{42920,42920,1},
{42922,42922,-42308},{42923,42923,-42319},{42924,42924,-42315},{42925,42925,-42305},{42926,42926,-42308},{42928,42928,-42258},
{42929,42929,-42282},{42930,42930,-42261},{42931,42931,928},{42932,42932,1},{42934,42934,1},{42936,42936,1},
{42938,42938,1},{42940,42940,1},{42942,42942,1},{42944,42944,1},{42946,42946,1},{42948,42948,-48},
{42949,42949,-42307},{42950,42950,-35384},{42951,42951,1},{42953,42953,1},{42955,42955,-42343},{42956,42956,1},
{42958,42958,1},{42960,42960,1},{42962,42962,1},{42964,42964,1},{42966,42966,1},{42968,42968,1},
{42970,42970,1},{42972,42972,-42561},{42997,42997,1},{65313,65338,32},{66560,66599,40},{66736,66771,40},
{66928,66938,39},{66940,66954,39},{66956,66962,39},{66964,66965,39},{68736,68786,64},{68944,68965,32},
{71840,71871,32},{93760,93791,32},{93856,93880,27},{125184,125217,34},
};

static bool ku_in(const uint32_t (*t)[2], size_t n, uint32_t c) {
    size_t lo = 0, hi = n;
    while (lo < hi) {
        size_t mid = (lo + hi) / 2;
        if (c < t[mid][0]) hi = mid;
        else if (c > t[mid][1]) lo = mid + 1;
        else return true;
    }
    return false;
}
#define KU_IN(t, c) ku_in(t, sizeof t / sizeof t[0], c)

static uint32_t ku_case(const int32_t (*t)[3], size_t n, uint32_t c) {
    size_t lo = 0, hi = n;
    while (lo < hi) {
        size_t mid = (lo + hi) / 2;
        if ((int32_t)c < t[mid][0]) hi = mid;
        else if ((int32_t)c > t[mid][1]) lo = mid + 1;
        else return (uint32_t)((int32_t)c + t[mid][2]);
    }
    return c;
}

/* One bit each: a letter, a number, white space, upper case, lower case. */
static kv kraw_char_class(kv ch) {
    uint32_t c = ch.u.c;
    int64_t r = 0;
    if (c < 0x80) {
        bool up = c >= 'A' && c <= 'Z', low = c >= 'a' && c <= 'z';
        if (up || low) r |= 1;
        if (c >= '0' && c <= '9') r |= 2;
        if ((c >= 9 && c <= 13) || c == ' ') r |= 4;
        if (up) r |= 8;
        if (low) r |= 16;
    } else {
        if (KU_IN(KU_ALPHA, c)) r |= 1;
        if (KU_IN(KU_NUMERIC, c)) r |= 2;
        if (KU_IN(KU_SPACE, c)) r |= 4;
        if (KU_IN(KU_UPPER, c)) r |= 8;
        if (KU_IN(KU_LOWER, c)) r |= 16;
    }
    return kint(r);
}

static kv kraw_char_case(kv ch, kv upper) {
    uint32_t c = ch.u.c;
    if (c < 0x80) {
        if (upper.u.t) return kchar(c >= 'a' && c <= 'z' ? c - 32 : c);
        return kchar(c >= 'A' && c <= 'Z' ? c + 32 : c);
    }
    if (upper.u.t) return kchar(ku_case(KU_TO_UPPER, sizeof KU_TO_UPPER / sizeof KU_TO_UPPER[0], c));
    return kchar(ku_case(KU_TO_LOWER, sizeof KU_TO_LOWER / sizeof KU_TO_LOWER[0], c));
}

/* ---- calls ---- */

static KLOCAL char *kstack_base;
#ifdef _WIN32
#define KSTACK_LIMIT (700 * 1024)
#else
#define KSTACK_LIMIT (6 * 1024 * 1024)
#endif

/* Reports deep recursion as a panic before the real stack runs out. */
static void kenter(void) {
    char here;
    ptrdiff_t used = kstack_base - &here;
    if (used < 0) used = -used;
    if (used > KSTACK_LIMIT) kpanic("stack overflow: the calls are nested too deep");
}

static kv kcall(kv f, kv *args);

/* ---- tasks ----
 *
 * A task is a block of four items: its result, whether it is done,
 * whether it was asked to stop, and the task that started it (0 when the
 * program itself did, or once it is done). A scope is a mark in the list
 * of the tasks its thread started: closing it waits for the ones after
 * the mark. One lock covers every task.
 *
 * At most KTHREADS_MAX tasks have a thread of their own at a time (the
 * variable MOU_THREADS changes it); one started beyond that runs where it
 * is started, before going on. */
#ifdef KTHREADS
enum { KTHREADS_MAX = 256 };
static pthread_mutex_t ktask_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t ktask_done = PTHREAD_COND_INITIALIZER;
static pthread_cond_t kchan_moved = PTHREAD_COND_INITIALIZER;
static int64_t kthreads_live, kthreads_max;
/* The task this thread runs, when it runs one. */
static KLOCAL kv ktask_self;
static KLOCAL bool ktask_inside;

/* Whether the task that runs here, or one that started it, was asked to
 * stop. With the lock taken. */
static bool ktask_stop(void) {
    if (!ktask_inside) return false;
    kv t = ktask_self;
    while (t.tag == T_TUPLE) {
        if (ITEMS(t.u.o)[2].u.i != 0) return true;
        t = ITEMS(t.u.o)[3];
    }
    return false;
}

static kv ktask_cancelled(void) {
    pthread_mutex_lock(&ktask_lock);
    bool stop = ktask_stop();
    pthread_mutex_unlock(&ktask_lock);
    return kbool(stop);
}

static kv ktask_cancel(kv task) {
    pthread_mutex_lock(&ktask_lock);
    ITEMS(task.u.o)[2] = kint(1);
    /* Whoever waits on a channel looks again. */
    pthread_cond_broadcast(&kchan_moved);
    pthread_mutex_unlock(&ktask_lock);
    krelease(task);
    return KUNIT;
}
static KLOCAL kv *ktasks;
static KLOCAL int64_t ktask_len, ktask_cap;

static kv ktask_open(void) { return kint(ktask_len); }

static void ktask_finish(kv task, kv result) {
    pthread_mutex_lock(&ktask_lock);
    ITEMS(task.u.o)[0] = result;
    ITEMS(task.u.o)[1] = kint(1);
    /* It lets go of who started it: a result may hold tasks. */
    kv parent = ITEMS(task.u.o)[3];
    ITEMS(task.u.o)[3] = kint(0);
    pthread_cond_broadcast(&ktask_done);
    pthread_mutex_unlock(&ktask_lock);
    krelease(parent);
}

static kv ktask_close(kv mark) {
    while (ktask_len > (int64_t)mark.u.i) {
        kv task = ktasks[--ktask_len];
        pthread_mutex_lock(&ktask_lock);
        while (ITEMS(task.u.o)[1].u.i == 0) pthread_cond_wait(&ktask_done, &ktask_lock);
        pthread_mutex_unlock(&ktask_lock);
        krelease(task);
    }
    return KUNIT;
}

/* What a thread runs: the task, with what it runs in the place of its result. */
static void *ktask_main(void *arg) {
    kv task = *(kv *)arg;
    char base;
    free(arg);
    kstack_base = &base;
    ktask_self = task;
    ktask_inside = true;
    kv A[1] = {KUNIT};
    kv result = kcall(ITEMS(task.u.o)[0], A);
    /* The tasks it started on a scope of another thread end before it does. */
    ktask_close(kint(0));
    free(ktasks);
    ktask_finish(task, result);
    krelease(task);
    kpool_drop();
    pthread_mutex_lock(&ktask_lock);
    kthreads_live--;
    pthread_mutex_unlock(&ktask_lock);
    return NULL;
}

static kv ktask_spawn(kv run) {
    kv items[4] = {run, kint(0), kint(0), ktask_inside ? kretain(ktask_self) : kint(0)};
    kv task = kagg(T_TUPLE, 0, 0, 4, items);
    if (ktask_len == ktask_cap) {
        ktask_cap = ktask_cap ? ktask_cap * 2 : 8;
        ktasks = (kv *)realloc(ktasks, (size_t)ktask_cap * sizeof(kv));
        if (!ktasks) kpanic("out of memory");
    }
    ktasks[ktask_len++] = kretain(task);
    kv *arg = (kv *)malloc(sizeof(kv));
    pthread_attr_t attr;
    pthread_t thread;
    bool started = false;
    pthread_mutex_lock(&ktask_lock);
    if (kthreads_max == 0) {
        const char *asked = getenv("MOU_THREADS");
        long n = asked ? strtol(asked, NULL, 10) : 0;
        kthreads_max = n > 0 ? n : KTHREADS_MAX;
    }
    bool room = kthreads_live < kthreads_max;
    if (room) kthreads_live++;
    pthread_mutex_unlock(&ktask_lock);
    if (room && !(arg && pthread_attr_init(&attr) == 0)) {
        pthread_mutex_lock(&ktask_lock);
        kthreads_live--;
        pthread_mutex_unlock(&ktask_lock);
    } else if (room) {
        *arg = kretain(task);
        pthread_attr_setstacksize(&attr, (size_t)KSTACK_LIMIT + 2 * 1024 * 1024);
        pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED);
        started = pthread_create(&thread, &attr, ktask_main, arg) == 0;
        pthread_attr_destroy(&attr);
        if (!started) {
            krelease(task);
            pthread_mutex_lock(&ktask_lock);
            kthreads_live--;
            pthread_mutex_unlock(&ktask_lock);
        }
    }
    if (!started) {
        /* No thread to be had: the task runs here, before going on. */
        free(arg);
        kv outer = ktask_self;
        bool was = ktask_inside;
        ktask_self = task;
        ktask_inside = true;
        kv A[1] = {KUNIT};
        kv result = kcall(run, A);
        ktask_self = outer;
        ktask_inside = was;
        ktask_finish(task, result);
    }
    return task;
}

static kv ktask_wait(kv task) {
    pthread_mutex_lock(&ktask_lock);
    while (ITEMS(task.u.o)[1].u.i == 0) pthread_cond_wait(&ktask_done, &ktask_lock);
    kv r = kretain(ITEMS(task.u.o)[0]);
    pthread_mutex_unlock(&ktask_lock);
    krelease(task);
    return r;
}
#else
/* A program that starts no tasks has none to wait for. */
static kv ktask_open(void) { return kint(0); }
static kv ktask_close(kv mark) { (void)mark; return KUNIT; }
static kv ktask_wait(kv task) { return task; }
static kv ktask_cancel(kv task) { krelease(task); return KUNIT; }
static kv ktask_cancelled(void) { return kbool(false); }
static bool ktask_stop(void) { return false; }
#endif

/* ---- channels ----
 *
 * A channel is a block: how many values it holds at most, how many it
 * holds now, where the first of them is, whether it is closed, and then
 * the values, in a ring. The lock of the tasks covers every channel. A
 * program that starts no tasks has nobody to wait for. */
#ifdef KTHREADS
#define KCHAN_LOCK() pthread_mutex_lock(&ktask_lock)
#define KCHAN_UNLOCK() pthread_mutex_unlock(&ktask_lock)
#define KCHAN_WAIT(what) pthread_cond_wait(&kchan_moved, &ktask_lock)
#define KCHAN_WAKE() pthread_cond_broadcast(&kchan_moved)
#else
#define KCHAN_LOCK() ((void)0)
#define KCHAN_UNLOCK() ((void)0)
#define KCHAN_WAIT(what) kpanic(what " would wait forever: this program starts no tasks")
#define KCHAN_WAKE() ((void)0)
#endif
enum { KCHAN_CAP, KCHAN_LEN, KCHAN_HEAD, KCHAN_CLOSED, KCHAN_ITEMS };

/* ---- values tasks share ----
 *
 * A `Mutex` is a block of two items: the value, and which thread is
 * updating it (0 when none is). An update takes the value out and puts
 * the new one back; whoever wants it in between waits. */
static KLOCAL char kself;
#define KME ((i128)(uintptr_t)&kself)

/* Waits, with the lock taken, until nobody is updating the value. */
static void kmutex_free(kv *it) {
    while (it[1].u.i != 0) {
        if (it[1].u.i == KME) {
            KCHAN_UNLOCK();
            kpanic("this `Mutex` is used inside its own `update`");
        }
        KCHAN_WAIT("a `Mutex`");
    }
}

static kv kmutex_new(kv value) {
    kv items[2] = {value, kint(0)};
    return kagg(T_TUPLE, 0, 0, 2, items);
}

static kv kmutex_get(kv m) {
    kv *it = ITEMS(m.u.o);
    KCHAN_LOCK();
    kmutex_free(it);
    kv r = kretain(it[0]);
    KCHAN_UNLOCK();
    krelease(m);
    return r;
}

static kv kmutex_set(kv m, kv value) {
    kv *it = ITEMS(m.u.o);
    KCHAN_LOCK();
    kmutex_free(it);
    kv old = it[0];
    it[0] = value;
    KCHAN_UNLOCK();
    krelease(old);
    krelease(m);
    return KUNIT;
}

static kv kmutex_take(kv m) {
    kv *it = ITEMS(m.u.o);
    KCHAN_LOCK();
    kmutex_free(it);
    it[1] = kint(KME);
    kv r = it[0];
    it[0] = KUNIT;
    KCHAN_UNLOCK();
    krelease(m);
    return r;
}

static kv kmutex_put(kv m, kv value) {
    kv *it = ITEMS(m.u.o);
    KCHAN_LOCK();
    it[0] = value;
    it[1] = kint(0);
    KCHAN_WAKE();
    KCHAN_UNLOCK();
    krelease(m);
    return KUNIT;
}

static kv kchan_new(kv cap) {
    if (cap.u.i < 1) kpanic("a channel holds at least one value");
    if (cap.u.i > ((i128)1 << 24)) kpanic("a channel cannot hold that many values");
    int64_t n = (int64_t)cap.u.i;
    kv v = kmk(T_TUPLE);
    v.u.o = kalloc((size_t)(KCHAN_ITEMS + n) * sizeof(kv), KCHAN_ITEMS + n);
    kv *it = ITEMS(v.u.o);
    it[KCHAN_CAP] = kint(n);
    it[KCHAN_LEN] = it[KCHAN_HEAD] = it[KCHAN_CLOSED] = kint(0);
    for (int64_t i = 0; i < n; i++) it[KCHAN_ITEMS + i] = KUNIT;
    return v;
}

static kv kchan_send(kv chan, kv value) {
    kv *it = ITEMS(chan.u.o);
    KCHAN_LOCK();
    while (it[KCHAN_LEN].u.i == it[KCHAN_CAP].u.i && it[KCHAN_CLOSED].u.i == 0) {
        if (ktask_stop()) {
            /* A task asked to stop does not wait: what it sends is dropped. */
            KCHAN_UNLOCK();
            krelease(value);
            krelease(chan);
            return KUNIT;
        }
        KCHAN_WAIT("`send`");
    }
    if (it[KCHAN_CLOSED].u.i != 0) {
        KCHAN_UNLOCK();
        kpanic("`send` on a closed channel");
    }
    int64_t cap = (int64_t)it[KCHAN_CAP].u.i;
    it[KCHAN_ITEMS + (int64_t)(it[KCHAN_HEAD].u.i + it[KCHAN_LEN].u.i) % cap] = value;
    it[KCHAN_LEN].u.i++;
    KCHAN_WAKE();
    KCHAN_UNLOCK();
    krelease(chan);
    return KUNIT;
}

static kv kchan_recv(kv chan) {
    kv *it = ITEMS(chan.u.o);
    KCHAN_LOCK();
    /* A task asked to stop does not wait: it finds the channel empty. */
    while (it[KCHAN_LEN].u.i == 0 && it[KCHAN_CLOSED].u.i == 0 && !ktask_stop()) KCHAN_WAIT("`recv`");
    bool got = it[KCHAN_LEN].u.i > 0;
    kv value = KUNIT;
    if (got) {
        int64_t head = (int64_t)it[KCHAN_HEAD].u.i;
        value = it[KCHAN_ITEMS + head];
        it[KCHAN_ITEMS + head] = KUNIT;
        it[KCHAN_HEAD].u.i = (head + 1) % (int64_t)it[KCHAN_CAP].u.i;
        it[KCHAN_LEN].u.i--;
        KCHAN_WAKE();
    }
    KCHAN_UNLOCK();
    krelease(chan);
    return got ? kbox(T_SOME, value) : KNONE;
}

static kv kchan_close(kv chan) {
    KCHAN_LOCK();
    ITEMS(chan.u.o)[KCHAN_CLOSED].u.i = 1;
    KCHAN_WAKE();
    KCHAN_UNLOCK();
    krelease(chan);
    return KUNIT;
}

static kv karena_close(kv mark) {
    uint32_t id = (uint32_t)mark.u.i;
    if (id == 0) return KUNIT;
    karena *a = karenas[id];
    karena_cur = a->outer;
    a->open = false;
    if (a->live == 0) karena_drop(id);
    return KUNIT;
}

/* ---- arrays lent to C ----
 *
 * A C function is given an array of numbers as the address of a copy of
 * them, laid out as C has them; what it leaves there is read back into a
 * new array when it may change them. */
enum { KB_I8, KB_I16, KB_I32, KB_I64, KB_ISIZE, KB_U8, KB_U16, KB_U32, KB_U64, KB_USIZE, KB_F32, KB_F64 };

static size_t kbuf_size(int kind) {
    switch (kind) {
    case KB_I8: case KB_U8: return 1;
    case KB_I16: case KB_U16: return 2;
    case KB_I32: case KB_U32: case KB_F32: return 4;
    case KB_ISIZE: return sizeof(ptrdiff_t);
    case KB_USIZE: return sizeof(size_t);
    default: return 8;
    }
}

static void *kbuf_make(kv a, int kind) {
    int64_t n = klen(a);
    size_t size = kbuf_size(kind);
    char *buf = (char *)malloc((size_t)n * size + 8);
    if (!buf) kpanic("out of memory");
    for (int64_t i = 0; i < n; i++) {
        kv v = kelem(a.u.o, i);
        char *at = buf + (size_t)i * size;
        switch (kind) {
        case KB_F64: { double x = v.u.f; memcpy(at, &x, sizeof x); break; }
        case KB_F32: { float x = v.u.g; memcpy(at, &x, sizeof x); break; }
        case KB_I8: case KB_U8: { uint8_t x = (uint8_t)kwide(v); memcpy(at, &x, sizeof x); break; }
        case KB_I16: case KB_U16: { uint16_t x = (uint16_t)kwide(v); memcpy(at, &x, sizeof x); break; }
        case KB_I32: case KB_U32: { uint32_t x = (uint32_t)kwide(v); memcpy(at, &x, sizeof x); break; }
        case KB_ISIZE: { ptrdiff_t x = (ptrdiff_t)kwide(v); memcpy(at, &x, sizeof x); break; }
        case KB_USIZE: { size_t x = (size_t)kwide(v); memcpy(at, &x, sizeof x); break; }
        default: { uint64_t x = (uint64_t)kwide(v); memcpy(at, &x, sizeof x); break; }
        }
    }
    return buf;
}

/* The `n` numbers at `buf` as an array. */
static kv kbuf_back(const void *buf, int64_t n, int kind) {
    size_t size = kbuf_size(kind);
    kv *items = (kv *)malloc((size_t)n * sizeof(kv) + 1);
    if (!items) kpanic("out of memory");
    for (int64_t i = 0; i < n; i++) {
        const char *at = (const char *)buf + (size_t)i * size;
        kv v = kmk(T_INT);
        switch (kind) {
        case KB_F64: { double x; memcpy(&x, at, sizeof x); v = kmk(T_F64); v.u.f = x; break; }
        case KB_F32: { float x; memcpy(&x, at, sizeof x); v = kmk(T_F32); v.u.g = x; break; }
        case KB_I8: { int8_t x; memcpy(&x, at, sizeof x); v.u.i = x; break; }
        case KB_U8: { uint8_t x; memcpy(&x, at, sizeof x); v.u.i = x; break; }
        case KB_I16: { int16_t x; memcpy(&x, at, sizeof x); v.u.i = x; break; }
        case KB_U16: { uint16_t x; memcpy(&x, at, sizeof x); v.u.i = x; break; }
        case KB_I32: { int32_t x; memcpy(&x, at, sizeof x); v.u.i = x; break; }
        case KB_U32: { uint32_t x; memcpy(&x, at, sizeof x); v.u.i = x; break; }
        case KB_I64: { int64_t x; memcpy(&x, at, sizeof x); v.u.i = x; break; }
        case KB_ISIZE: { ptrdiff_t x; memcpy(&x, at, sizeof x); v.u.i = x; break; }
        case KB_USIZE: { size_t x; memcpy(&x, at, sizeof x); v = kint((i128)x); break; }
        default: { uint64_t x; memcpy(&x, at, sizeof x); v = kint((i128)x); break; }
        }
        items[i] = v;
    }
    kv r = kagg(T_ARRAY, 0, 0, n, items);
    free(items);
    return r;
}

/* ---- files ---- */

#include <dirent.h>
#include <sys/stat.h>

static kv kio_read_bytes(kv path) {
    FILE *f = fopen(kbytes(path), "rb");
    krelease(path);
    if (!f) return kos_error();
    ksb sb = {0};
    char buf[4096];
    size_t n;
    errno = 0;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) ksb_put(&sb, buf, n);
    if (ferror(f)) {
        kv r = kos_error();
        fclose(f);
        free(sb.p);
        return r;
    }
    fclose(f);
    kv r = kbox(T_OK, kbuf_back(sb.p ? sb.p : "", (int64_t)sb.len, KB_U8));
    free(sb.p);
    return r;
}

static kv kio_write_bytes(kv path, kv data, kv append) {
    FILE *f = fopen(kbytes(path), append.u.t ? "ab" : "wb");
    kv r;
    if (!f) {
        r = kos_error();
    } else {
        size_t n = (size_t)klen(data);
        void *buf = kbuf_make(data, KB_U8);
        bool ok = fwrite(buf, 1, n, f) == n;
        free(buf);
        ok = fclose(f) == 0 && ok;
        r = ok ? kbox(T_OK, KUNIT) : kos_error();
    }
    krelease(path);
    krelease(data);
    return r;
}

static int kname_cmp(const void *a, const void *b) { return strcmp(*(char *const *)a, *(char *const *)b); }

/* The names in a directory, in order. A name that is not text is left out. */
static kv kio_list_dir(kv path) {
    DIR *d = opendir(kbytes(path));
    krelease(path);
    if (!d) return kos_error();
    char **names = NULL;
    size_t n = 0, cap = 0;
    struct dirent *e;
    while ((e = readdir(d))) {
        const char *s = e->d_name;
        if (!strcmp(s, ".") || !strcmp(s, "..") || !kutf8((const unsigned char *)s, strlen(s))) continue;
        if (n == cap) {
            cap = cap ? cap * 2 : 16;
            names = (char **)realloc(names, cap * sizeof *names);
            if (!names) kpanic("out of memory");
        }
        names[n] = strdup(s);
        if (!names[n++]) kpanic("out of memory");
    }
    closedir(d);
    if (n) qsort(names, n, sizeof *names, kname_cmp);
    kv v = karray_new(P_NONE, (int64_t)n);
    for (size_t i = 0; i < n; i++) {
        ITEMS(v.u.o)[i] = kstr(names[i], strlen(names[i]));
        free(names[i]);
    }
    free(names);
    return kbox(T_OK, v);
}

/* 0 when there is nothing there, 1 a file, 2 a directory, 3 anything else. */
static kv kio_file_kind(kv path) {
    struct stat st;
    int64_t kind = 0;
    if (stat(kbytes(path), &st) == 0) kind = S_ISREG(st.st_mode) ? 1 : S_ISDIR(st.st_mode) ? 2 : 3;
    krelease(path);
    return kint(kind);
}

static kv kio_make_dir(kv path) {
    kv r = mkdir(kbytes(path), 0777) == 0 ? kbox(T_OK, KUNIT) : kos_error();
    krelease(path);
    return r;
}

static kv kio_remove(kv path) {
    struct stat st;
    bool dir = lstat(kbytes(path), &st) == 0 && S_ISDIR(st.st_mode);
    kv r = (dir ? rmdir(kbytes(path)) : unlink(kbytes(path))) == 0 ? kbox(T_OK, KUNIT) : kos_error();
    krelease(path);
    return r;
}

static kv kio_rename(kv from, kv to) {
    kv r = rename(kbytes(from), kbytes(to)) == 0 ? kbox(T_OK, KUNIT) : kos_error();
    krelease(from);
    krelease(to);
    return r;
}

#include <arpa/inet.h>
#include <netdb.h>
#include <sys/socket.h>

/* The addresses of IPv4 a name stands for, as text, each one once. */
static kv kio_resolve(kv host) {
    struct addrinfo hints, *found = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    int code = getaddrinfo(kbytes(host), NULL, &hints, &found);
    krelease(host);
    if (code != 0) {
        char buf[300];
        snprintf(buf, sizeof buf, "failed to lookup address information: %s", gai_strerror(code));
        return kfailed(buf);
    }
    kv v = karray_new(P_NONE, 0);
    /* The addresses of IPv4 first, and then the ones of IPv6. */
    for (int six = 0; six < 2; six++)
    for (struct addrinfo *a = found; a; a = a->ai_next) {
        char name[INET6_ADDRSTRLEN];
        if (a->ai_family != (six ? AF_INET6 : AF_INET)) continue;
        const void *bytes = six ? (const void *)&((struct sockaddr_in6 *)a->ai_addr)->sin6_addr : (const void *)&((struct sockaddr_in *)a->ai_addr)->sin_addr;
        if (!inet_ntop(a->ai_family, bytes, name, sizeof name)) continue;
        bool seen = false;
        for (int64_t i = 0; i < klen(v) && !seen; i++) {
            kv had = kelem(v.u.o, i);
            seen = (size_t)klen(had) == strlen(name) && !memcmp(kbytes(had), name, strlen(name));
        }
        if (!seen) kpush(&v, 0, NULL, kstr(name, strlen(name)));
    }
    freeaddrinfo(found);
    return kbox(T_OK, v);
}

/* The files the program has open. A file is its place here, and a closed
   one leaves its place to the next that is opened. */
static FILE **kfiles;
static size_t kfiles_n;
#ifdef KTHREADS
static pthread_mutex_t kfiles_lock = PTHREAD_MUTEX_INITIALIZER;
#define KFILES_LOCK() pthread_mutex_lock(&kfiles_lock)
#define KFILES_UNLOCK() pthread_mutex_unlock(&kfiles_lock)
#else
#define KFILES_LOCK() ((void)0)
#define KFILES_UNLOCK() ((void)0)
#endif

static kv kio_fopen(kv path, kv mode) {
    int64_t m = (int64_t)ktakei(mode);
    FILE *f = fopen(kbytes(path), m == 0 ? "rb" : m == 1 ? "wb" : "ab");
    krelease(path);
    if (!f) return kos_error();
    /* What is written is in the file at once, as in the interpreter. */
    setvbuf(f, NULL, _IONBF, 0);
    KFILES_LOCK();
    size_t at = 0;
    while (at < kfiles_n && kfiles[at]) at++;
    if (at == kfiles_n) {
        kfiles = (FILE **)realloc(kfiles, (kfiles_n + 1) * sizeof *kfiles);
        if (!kfiles) kpanic("out of memory");
        kfiles_n++;
    }
    kfiles[at] = f;
    KFILES_UNLOCK();
    return kbox(T_OK, kint((int64_t)at));
}

static FILE *kfile_at(kv file, bool close) {
    i128 h = ktakei(file);
    FILE *f = NULL;
    KFILES_LOCK();
    if (h >= 0 && h < (i128)kfiles_n) {
        f = kfiles[(size_t)h];
        if (close) kfiles[(size_t)h] = NULL;
    }
    KFILES_UNLOCK();
    return f;
}

static kv kio_fread(kv file, kv max) {
    FILE *f = kfile_at(file, false);
    i128 want = ktakei(max);
    if (!f) return kfailed("the file is closed");
    ksb sb = {0};
    char buf[4096];
    errno = 0;
    while (want > 0) {
        size_t ask = want < (i128)sizeof buf ? (size_t)want : sizeof buf;
        size_t n = fread(buf, 1, ask, f);
        if (n) ksb_put(&sb, buf, n);
        want -= (i128)n;
        if (n < ask) break;
    }
    if (ferror(f)) {
        kv r = kos_error();
        clearerr(f);
        free(sb.p);
        return r;
    }
    clearerr(f);
    kv r = kbox(T_OK, kbuf_back(sb.p ? sb.p : "", (int64_t)sb.len, KB_U8));
    free(sb.p);
    return r;
}

static kv kio_fwrite(kv file, kv data) {
    FILE *f = kfile_at(file, false);
    if (!f) {
        krelease(data);
        return kfailed("the file is closed");
    }
    size_t n = (size_t)klen(data);
    void *buf = kbuf_make(data, KB_U8);
    errno = 0;
    bool ok = fwrite(buf, 1, n, f) == n;
    free(buf);
    krelease(data);
    if (!ok) {
        kv r = kos_error();
        clearerr(f);
        return r;
    }
    return kbox(T_OK, KUNIT);
}

static kv kio_fclose(kv file) {
    FILE *f = kfile_at(file, true);
    if (!f) return kfailed("the file is closed");
    return fclose(f) == 0 ? kbox(T_OK, KUNIT) : kos_error();
}

/* Calls a function value. Takes ownership of `f`; the callee leaves the
 * final values of its parameters in `args`. */
static kv kcall(kv f, kv *args) {
    if (f.tag != T_FN && f.tag != T_CLOSURE) kinternal("a call to a value that is not a function");
    kv r = kdispatch(f.a, args, f);
    krelease(f);
    return r;
}

static int kfinish(kv r) {
    fflush(stdout);
    if (r.tag == T_ERR) {
        ksb sb = {0};
        kshow(&sb, ITEMS(r.u.o)[0], false);
        fprintf(stderr, "%s: `main` returned an error: %s\n", KFILE, sb.p);
        return 1;
    }
    kv inner = r.tag == T_OK ? ITEMS(r.u.o)[0] : r;
    int status = inner.tag == T_INT ? (int)(inner.u.i & 0xFF) : 0;
    krelease(r);
    return status;
}
