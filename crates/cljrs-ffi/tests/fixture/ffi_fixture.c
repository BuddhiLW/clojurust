/* C fixture for clojure.rust.ffi; built into a shared object by tests/common. */
#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int add_i(int a, int b) { return a + b; }

long add_l(long a, long b) { return a + b; }

double scale(double x, long k) { return x * (double)k; }

/* Each argument lands in its own decimal digit, so a swap shows. */
long mixed(long a, double b, long c, double d, long e, double f) {
    return a * 1 + (long)b * 10 + c * 100 + (long)d * 1000 + e * 10000 + (long)f * 100000;
}

int neg(void) { return -7; }

static char greet_buf[256];

const char *greet(const char *name) {
    snprintf(greet_buf, sizeof greet_buf, "hi %s", name ? name : "(null)");
    return greet_buf;
}

char *dup_upper(const char *s) {
    size_t n = strlen(s);
    char *out = malloc(n + 1);
    for (size_t i = 0; i <= n; i++) out[i] = (char)toupper((unsigned char)s[i]);
    return out;
}

void fixture_free(void *p) { free(p); }

long sum_bytes(const unsigned char *p, long n) {
    long sum = 0;
    for (long i = 0; i < n; i++) sum += p[i];
    return sum;
}

void *null_ptr(void) { return NULL; }

long six_ints(long a, long b, long c, long d, long e, long f) {
    return a * 1 + b * 10 + c * 100 + d * 1000 + e * 10000 + f * 100000;
}

double eight_doubles(double a, double b, double c, double d,
                     double e, double f, double g, double h) {
    return a * 1 + b * 10 + c * 100 + d * 1000 + e * 10000 + f * 100000 + g * 1000000 + h * 10000000;
}

int is_null(const char *s) { return s == NULL; }

static unsigned char raw[4] = {1, 2, 254, 255};

const unsigned char *raw_bytes(void) { return raw; }
