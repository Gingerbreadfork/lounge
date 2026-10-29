// Separable resampling of RGBA8 pictures in Q14 fixed point: a horizontal pass over the rows the
// output needs, then a vertical pass over those. The inner loops use NEON where the target has it
// and the same arithmetic in plain C elsewhere, so every platform produces the same bytes.
#include <math.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#if defined(__ARM_NEON) || defined(__ARM_NEON__)
#include <arm_neon.h>
#define LOUNGE_NEON 1
#endif

#define Q 14
#define ONE (1 << Q)
#define PI 3.14159265358979323846

enum { KERNEL_LANCZOS3 = 0, KERNEL_MITCHELL = 1 };

static double lanczos3(double x) {
    if (x <= -3.0 || x >= 3.0) return 0.0;
    if (x == 0.0) return 1.0;
    double px = PI * x;
    return 3.0 * sin(px) * sin(px / 3.0) / (px * px);
}

// Mitchell-Netravali with B and C both a third.
static double mitchell(double x) {
    x = fabs(x);
    if (x < 1.0) return (7.0 * x * x * x - 12.0 * x * x + 16.0 / 3.0) / 6.0;
    if (x < 2.0) return (-7.0 / 3.0 * x * x * x + 12.0 * x * x - 20.0 * x + 32.0 / 3.0) / 6.0;
    return 0.0;
}

// One axis: for every output index, where its taps start in the source, how many there are, and
// their weights, which sum to exactly ONE.
typedef struct {
    int32_t *start;
    int32_t *count;
    int16_t *weights;
    int32_t max_taps;
} axis_t;

static int axis_free(axis_t *a) {
    free(a->start);
    free(a->count);
    free(a->weights);
    return -1;
}

static int axis_plan(axis_t *a, uint32_t n_src, double offset, double extent, uint32_t n_out, int kernel) {
    double radius = kernel == KERNEL_LANCZOS3 ? 3.0 : 2.0;
    double scale = extent / (double)n_out;
    double stretch = scale > 1.0 ? scale : 1.0;
    double support = radius * stretch;
    a->max_taps = (int32_t)ceil(2.0 * support) + 2;
    a->start = calloc(n_out, sizeof(int32_t));
    a->count = calloc(n_out, sizeof(int32_t));
    a->weights = calloc((size_t)n_out * (size_t)a->max_taps, sizeof(int16_t));
    if (!a->start || !a->count || !a->weights) return axis_free(a);
    double *raw = malloc(sizeof(double) * (size_t)a->max_taps);
    if (!raw) return axis_free(a);
    for (uint32_t i = 0; i < n_out; i++) {
        double center = offset + ((double)i + 0.5) * scale;
        int32_t lo = (int32_t)ceil(center - support - 0.5);
        int32_t hi = (int32_t)floor(center + support - 0.5);
        if (lo < 0) lo = 0;
        if (hi > (int32_t)n_src - 1) hi = (int32_t)n_src - 1;
        if (hi < lo) {
            int32_t at = (int32_t)floor(center);
            if (at < 0) at = 0;
            if (at > (int32_t)n_src - 1) at = (int32_t)n_src - 1;
            lo = hi = at;
        }
        int32_t count = hi - lo + 1;
        if (count > a->max_taps) count = a->max_taps;
        double sum = 0.0;
        for (int32_t k = 0; k < count; k++) {
            double x = ((double)(lo + k) + 0.5 - center) / stretch;
            raw[k] = kernel == KERNEL_LANCZOS3 ? lanczos3(x) : mitchell(x);
            sum += raw[k];
        }
        if (sum == 0.0) {
            raw[count / 2] = 1.0;
            sum = 1.0;
        }
        int16_t *w = a->weights + (size_t)i * (size_t)a->max_taps;
        int32_t total = 0, largest = 0;
        for (int32_t k = 0; k < count; k++) {
            int32_t q = (int32_t)lround(raw[k] / sum * ONE);
            w[k] = (int16_t)q;
            total += q;
            if (abs(q) > abs(w[largest])) largest = k;
        }
        w[largest] = (int16_t)(w[largest] + (ONE - total));
        a->start[i] = lo;
        a->count[i] = count;
    }
    free(raw);
    return 0;
}

static inline uint8_t narrow(int32_t acc) {
    int32_t v = (acc + (1 << (Q - 1))) >> Q;
    return v < 0 ? 0 : v > 255 ? 255 : (uint8_t)v;
}

// One row of the horizontal pass: `src_row` resampled into `dst_row` of `n_out` pixels.
static void pass_h(const uint8_t *src_row, uint8_t *dst_row, uint32_t n_out, const axis_t *a) {
    for (uint32_t i = 0; i < n_out; i++) {
        const uint8_t *sp = src_row + 4 * (size_t)a->start[i];
        const int16_t *w = a->weights + (size_t)i * (size_t)a->max_taps;
        int32_t count = a->count[i];
#ifdef LOUNGE_NEON
        int32x4_t acc = vdupq_n_s32(0);
        for (int32_t k = 0; k < count; k++) {
            uint32_t px;
            memcpy(&px, sp + 4 * (size_t)k, 4);
            int16x4_t s = vreinterpret_s16_u16(vget_low_u16(vmovl_u8(vreinterpret_u8_u32(vdup_n_u32(px)))));
            acc = vmlal_n_s16(acc, s, w[k]);
        }
        int16x4_t n = vqrshrn_n_s32(acc, Q);
        uint8x8_t o = vqmovun_s16(vcombine_s16(n, n));
        uint32_t out = vget_lane_u32(vreinterpret_u32_u8(o), 0);
        memcpy(dst_row + 4 * (size_t)i, &out, 4);
#else
        int32_t acc[4] = {0, 0, 0, 0};
        for (int32_t k = 0; k < count; k++) {
            const uint8_t *p = sp + 4 * (size_t)k;
            for (int c = 0; c < 4; c++) acc[c] += (int32_t)p[c] * w[k];
        }
        for (int c = 0; c < 4; c++) dst_row[4 * (size_t)i + c] = narrow(acc[c]);
#endif
    }
}

// One row of the vertical pass: the taps' rows of `inter`, each `width` pixels, into `dst_row`.
static void pass_v(const uint8_t *inter, uint32_t width, int32_t first_row, int32_t count, const int16_t *w, uint8_t *dst_row) {
    uint32_t x = 0;
#ifdef LOUNGE_NEON
    for (; x + 4 <= width; x += 4) {
        int32x4_t a0 = vdupq_n_s32(0), a1 = a0, a2 = a0, a3 = a0;
        for (int32_t k = 0; k < count; k++) {
            const uint8_t *ip = inter + ((size_t)(first_row + k) * width + x) * 4;
            uint8x16_t p = vld1q_u8(ip);
            int16x8_t lo = vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(p)));
            int16x8_t hi = vreinterpretq_s16_u16(vmovl_u8(vget_high_u8(p)));
            a0 = vmlal_n_s16(a0, vget_low_s16(lo), w[k]);
            a1 = vmlal_n_s16(a1, vget_high_s16(lo), w[k]);
            a2 = vmlal_n_s16(a2, vget_low_s16(hi), w[k]);
            a3 = vmlal_n_s16(a3, vget_high_s16(hi), w[k]);
        }
        uint8x8_t o01 = vqmovun_s16(vcombine_s16(vqrshrn_n_s32(a0, Q), vqrshrn_n_s32(a1, Q)));
        uint8x8_t o23 = vqmovun_s16(vcombine_s16(vqrshrn_n_s32(a2, Q), vqrshrn_n_s32(a3, Q)));
        vst1q_u8(dst_row + 4 * (size_t)x, vcombine_u8(o01, o23));
    }
#endif
    for (; x < width; x++) {
        int32_t acc[4] = {0, 0, 0, 0};
        for (int32_t k = 0; k < count; k++) {
            const uint8_t *p = inter + ((size_t)(first_row + k) * width + x) * 4;
            for (int c = 0; c < 4; c++) acc[c] += (int32_t)p[c] * w[k];
        }
        for (int c = 0; c < 4; c++) dst_row[4 * (size_t)x + c] = narrow(acc[c]);
    }
}

// Resamples the (`left`, `top`, `cw`, `ch`) region of an `sw`×`sh` RGBA8 picture into `dst`,
// `tw`×`th`, with `kernel` 0 for Lanczos3 or 1 for Mitchell. Returns 0, or -1 when out of memory.
int lounge_resize_rgba(const uint8_t *src, uint32_t sw, uint32_t sh, double left, double top, double cw, double ch, uint8_t *dst, uint32_t tw, uint32_t th, int kernel) {
    if (sw == 0 || sh == 0 || tw == 0 || th == 0) return -1;
    axis_t h = {0}, v = {0};
    if (axis_plan(&h, sw, left, cw, tw, kernel) != 0) return -1;
    if (axis_plan(&v, sh, top, ch, th, kernel) != 0) return axis_free(&h);
    int32_t row_base = v.start[0], row_end = v.start[0] + v.count[0];
    for (uint32_t j = 1; j < th; j++) {
        if (v.start[j] < row_base) row_base = v.start[j];
        if (v.start[j] + v.count[j] > row_end) row_end = v.start[j] + v.count[j];
    }
    uint32_t rows = (uint32_t)(row_end - row_base);
    uint8_t *inter = malloc((size_t)rows * (size_t)tw * 4);
    if (!inter) {
        axis_free(&h);
        axis_free(&v);
        return -1;
    }
    for (uint32_t r = 0; r < rows; r++) {
        pass_h(src + (size_t)(row_base + (int32_t)r) * sw * 4, inter + (size_t)r * tw * 4, tw, &h);
    }
    for (uint32_t j = 0; j < th; j++) {
        pass_v(inter, tw, v.start[j] - row_base, v.count[j], v.weights + (size_t)j * (size_t)v.max_taps, dst + (size_t)j * tw * 4);
    }
    free(inter);
    axis_free(&h);
    axis_free(&v);
    return 0;
}
