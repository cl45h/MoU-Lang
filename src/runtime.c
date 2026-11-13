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

enum { T_UNIT, T_INT, T_F64, T_F32, T_BOOL, T_CHAR, T_STR, T_TUPLE, T_STRUCT, T_ENUM, T_ARRAY, T_NONE, T_SOME, T_OK, T_ERR, T_FN, T_CLOSURE };

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

typedef struct kv {
    uint8_t tag;
    uint32_t a, b; /* struct index; enum index and variant; function id */
    union {
        i128 i;
        double f;
        float g;
        bool t;
        uint32_t c;
        kobj *o;
    } u;
} kv;

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

static kv kint(i128 i) {
    kv v = kmk(T_INT);
    v.u.i = i;
    return v;
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
    case T_INT: return v.u.i >= INT64_MIN && v.u.i <= INT64_MAX ? P_I64 : P_NONE;
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
    case KF_U64: v = kmk(T_INT); v.u.i = (i128)(uint64_t)*slot; return v;
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
    default: *slot = (int64_t)v.u.i; break;
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
    if (v->tag != T_INT) kinternal("an index that is not an integer");
    if (v->u.i < 0) kpanicf("index ", *v, " is negative");
    return v->u.i > INT64_MAX ? INT64_MAX : (int64_t)v->u.i;
}

static int64_t kidxc(i128 i) {
    kv v = kint(i);
    return kidx(&v);
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
    if (a.tag != T_ARRAY || i.tag != T_INT) kinternal("`get` on a value that is not an array");
    kv r = kmk(T_NONE);
    if (i.u.i >= 0 && i.u.i < klen(a)) {
        kv item = kown(a.u.o, (int64_t)i.u.i);
        r = kagg(T_SOME, 0, 0, 1, &item);
    }
    krelease(a);
    return r;
}

/* `pop` and `get` of an array of numbers or booleans, for whoever keeps
   the optional without a block: whether there is an item, and the item. */
static bool kpop_raw(kv *root, int n, const kstep *steps, kv *item) {
    kv *v = karray_at(root, n, steps, "`pop` on a value that is not an array");
    kobj *o = v->u.o;
    if (o->len == 0) return false;
    o->len--;
    *item = kelem(o, o->len);
    return true;
}

static bool kget_raw(kv a, kv i, kv *item) {
    if (a.tag != T_ARRAY || i.tag != T_INT) kinternal("`get` on a value that is not an array");
    bool has = i.u.i >= 0 && i.u.i < klen(a);
    if (has) *item = kelem(a.u.o, (int64_t)i.u.i);
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
    if (KRARE(o->pack != P_I64)) return kelem(o, k).u.i;
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
    if (v->u.o->pack >= P_REC) krec_store(v->u.o, k, f, value);
    else kset(kitem_field_slow(v, k, f, true), value);
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
    if (KRARE(o->pack < P_REC)) return kitem_field_slow(v, k, f, false)->u.i;
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
        if (a.tag == T_INT && b.tag == T_INT) r = kint(op == O_AND ? a.u.i & b.u.i : op == O_OR ? a.u.i | b.u.i : a.u.i ^ b.u.i);
        else if (a.tag == T_BOOL && b.tag == T_BOOL) r = kbool(op == O_AND ? a.u.t & b.u.t : op == O_OR ? a.u.t | b.u.t : a.u.t ^ b.u.t);
        else kinternal("a bit operator applied to the wrong kind of value");
    } else if (a.tag == T_INT && b.tag == T_INT && k >= 0) r = kint(kint_op(op, k, a.u.i, b.u.i));
    else if (a.tag == T_F64 && b.tag == T_F64) r = kf64(kfloat_op(op, a.u.f, b.u.f));
    else if (a.tag == T_F32 && b.tag == T_F32) r = kf32((float)kfloat_op(op, (double)a.u.g, (double)b.u.g));
    else kinternal("an arithmetic operator applied to the wrong kind of value");
    krelease(a);
    krelease(b);
    return r;
}

enum { U_NEG, U_NOT, U_BITNOT };

static kv kun(int op, int k, kv v) {
    if (op == U_NEG && v.tag == T_INT && k >= 0) return kint(kint_op(O_SUB, k, 0, v.u.i));
    if (op == U_NEG && v.tag == T_F64) return kf64(-v.u.f);
    if (op == U_NEG && v.tag == T_F32) return kf32(-v.u.g);
    if (op == U_NOT && v.tag == T_BOOL) return kbool(!v.u.t);
    if (op == U_BITNOT && v.tag == T_INT && k >= 0) {
        i128 lo, hi;
        kbounds(k, &lo, &hi);
        if (lo < 0) return kint(~v.u.i);
        if (k == K_U128) kpanic(U128_LIMIT);
        return kint(hi - v.u.i);
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
    case T_INT: i = v.u.i; break;
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
    return kint(i);
}

static kv kcast_float(kv v, bool single) {
    double f = 0;
    if (v.tag == T_INT) f = (double)v.u.i;
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
        if (v.tag != T_INT) break;
        if (v.u.i < lo || v.u.i > hi) {
            ksb sb = {0};
            ksb_str(&sb, "the value ");
            ksb_i128(&sb, v.u.i);
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
    if (n.tag != T_INT) kinternal("an array length that is not an integer");
    if (n.u.i < 0 || n.u.i > 100000000) kpanicf("an array of length ", n, " cannot be created");
    int64_t len = (int64_t)n.u.i;
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
    if (i.tag != T_INT) kinternal("a range bound that is not an integer");
    if (i.u.i < 0) kpanicf("the range bound ", i, " is negative");
    return i.u.i > INT64_MAX - 1 ? INT64_MAX - 1 : (int64_t)i.u.i;
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
    if (bound.tag != T_INT) kinternal("a range bound that is not an integer");
    return i < bound.u.i || (i == bound.u.i && ktruth(krange_part(*r, 2)));
}

static bool krange_last_int(const kv *r, i128 i) {
    kv hi = krange_part(*r, 1);
    if (hi.tag != T_SOME || !ktruth(krange_part(*r, 2))) return false;
    kv bound = ITEMS(hi.u.o)[0];
    return bound.tag == T_INT && bound.u.i == i;
}

static kv krange_last(kv r, kv i) {
    kv hi = krange_part(r, 1);
    bool last = ktruth(krange_part(r, 2)) && hi.tag == T_SOME && keq(ITEMS(hi.u.o)[0], i);
    krelease(r);
    krelease(i);
    return kbool(last);
}

static kv ksucc(kv v, int k) {
    if (v.tag == T_INT && k >= 0) return kint(kint_op(O_ADD, k, v.u.i, 1));
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
} kcap;
#ifdef KTHREADS
static pthread_mutex_t kcap_lock = PTHREAD_MUTEX_INITIALIZER;
#define KCAP_LOCK() pthread_mutex_lock(&kcap_lock)
#define KCAP_UNLOCK() pthread_mutex_unlock(&kcap_lock)
#else
#define KCAP_LOCK() ((void)0)
#define KCAP_UNLOCK() ((void)0)
#endif

static kv kcapture_open(void) {
    KCAP_LOCK();
    if (kcap.depth == KCAPTURE_MAX) kpanic("too many `with io` blocks, one inside another");
    kcap.marks[kcap.depth++] = kcap.n;
    KCAP_UNLOCK();
    return KUNIT;
}

/* Keeps a line when a block is open. Takes the text when it does. */
static bool kcapture_line(char *text, size_t len) {
    if (!kcap.depth) return false;
    KCAP_LOCK();
    if (!kcap.depth) {
        KCAP_UNLOCK();
        return false;
    }
    if (kcap.n == kcap.cap) {
        kcap.cap = kcap.cap ? kcap.cap * 2 : 16;
        kcap.lines = (char **)realloc(kcap.lines, kcap.cap * sizeof(char *));
        kcap.lens = (size_t *)realloc(kcap.lens, kcap.cap * sizeof(size_t));
        if (!kcap.lines || !kcap.lens) kpanic("out of memory");
    }
    kcap.lines[kcap.n] = text;
    kcap.lens[kcap.n++] = len;
    KCAP_UNLOCK();
    return true;
}

/* Ends the innermost block: its lines go after the ones `old` has. */
static kv kcapture_close(kv old) {
    KCAP_LOCK();
    size_t from = kcap.depth ? kcap.marks[--kcap.depth] : kcap.n;
    size_t to = kcap.n;
    kcap.n = from;
    char **lines = NULL;
    size_t *lens = NULL;
    if (to > from) {
        /* A copy of the entries, so that the list can go on being used. */
        lines = (char **)malloc((to - from) * sizeof(char *));
        lens = (size_t *)malloc((to - from) * sizeof(size_t));
        if (!lines || !lens) kpanic("out of memory");
        memcpy(lines, kcap.lines + from, (to - from) * sizeof(char *));
        memcpy(lens, kcap.lens + from, (to - from) * sizeof(size_t));
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

static kv kprint(kv v) {
    ksb sb = {0};
    kshow(&sb, v, true);
    if (kcapture_line(sb.p, sb.len)) {
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
    ksb_str(&sb, "\n");
    fflush(stdout);
    bool ok = fwrite(sb.p, 1, sb.len, stderr) == sb.len;
    free(sb.p);
    krelease(v);
    return ok ? kbox(T_OK, KUNIT) : kfailed("write error");
}

/* The next line of the input without its end, or `None` when there is none. */
static kv kio_read_line(void) {
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

static kv kio_args(void) {
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
    uint64_t z = (uint64_t)(int64_t)n.u.i + 0x9e3779b97f4a7c15u;
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
static struct KOU KOU_get(kv v) { return v.tag == T_SOME ? (struct KOU){true, (uint64_t)ITEMS(v.u.o)[0].u.i} : (struct KOU){0}; }
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
    i128 i = n.u.i;
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
        case KB_I8: case KB_U8: { uint8_t x = (uint8_t)v.u.i; memcpy(at, &x, sizeof x); break; }
        case KB_I16: case KB_U16: { uint16_t x = (uint16_t)v.u.i; memcpy(at, &x, sizeof x); break; }
        case KB_I32: case KB_U32: { uint32_t x = (uint32_t)v.u.i; memcpy(at, &x, sizeof x); break; }
        case KB_ISIZE: { ptrdiff_t x = (ptrdiff_t)v.u.i; memcpy(at, &x, sizeof x); break; }
        case KB_USIZE: { size_t x = (size_t)v.u.i; memcpy(at, &x, sizeof x); break; }
        default: { uint64_t x = (uint64_t)v.u.i; memcpy(at, &x, sizeof x); break; }
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
        case KB_USIZE: { size_t x; memcpy(&x, at, sizeof x); v.u.i = (i128)x; break; }
        default: { uint64_t x; memcpy(&x, at, sizeof x); v.u.i = (i128)x; break; }
        }
        items[i] = v;
    }
    kv r = kagg(T_ARRAY, 0, 0, n, items);
    free(items);
    return r;
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
