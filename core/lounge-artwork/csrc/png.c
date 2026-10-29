// A PNG to RGBA8 through libpng, read from memory. libpng reports errors by a long jump, which
// stays inside these functions.
#include <png.h>
#include <setjmp.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    const uint8_t *data;
    size_t len;
    size_t pos;
} reader_t;

static void read_bytes(png_structp png, png_bytep out, size_t n) {
    reader_t *r = png_get_io_ptr(png);
    if (n > r->len - r->pos) png_error(png, "truncated");
    memcpy(out, r->data + r->pos, n);
    r->pos += n;
}

static void quiet(png_structp png, png_const_charp message) {
    (void)png;
    (void)message;
}

// The size of a PNG from its header, or -1 when the bytes are not one.
int lounge_png_size(const uint8_t *data, size_t len, uint32_t *w, uint32_t *h) {
    if (len < 24 || png_sig_cmp(data, 0, 8) != 0 || memcmp(data + 12, "IHDR", 4) != 0) return -1;
    *w = ((uint32_t)data[16] << 24) | ((uint32_t)data[17] << 16) | ((uint32_t)data[18] << 8) | data[19];
    *h = ((uint32_t)data[20] << 24) | ((uint32_t)data[21] << 16) | ((uint32_t)data[22] << 8) | data[23];
    return (*w > 0 && *h > 0 && *w <= 0x7fffffff && *h <= 0x7fffffff) ? 0 : -1;
}

// Decodes a `w`×`h` PNG into `out`, which holds w*h*4 bytes, as RGBA8: palettes and transparency
// expanded, grey widened, 16 bits narrowed, an opaque alpha added where there is none. Returns 0,
// or -1 when the picture will not decode.
int lounge_png_decode(const uint8_t *data, size_t len, uint8_t *out, uint32_t w, uint32_t h) {
    png_structp png = png_create_read_struct(PNG_LIBPNG_VER_STRING, NULL, NULL, quiet);
    if (!png) return -1;
    png_infop info = png_create_info_struct(png);
    if (!info) {
        png_destroy_read_struct(&png, NULL, NULL);
        return -1;
    }
    png_bytep *rows = NULL;
    if (setjmp(png_jmpbuf(png))) {
        free(rows);
        png_destroy_read_struct(&png, &info, NULL);
        return -1;
    }
    reader_t reader = {data, len, 0};
    png_set_read_fn(png, &reader, read_bytes);
    png_read_info(png, info);
    if (png_get_image_width(png, info) != w || png_get_image_height(png, info) != h) png_error(png, "size");
    png_set_expand(png);
    png_set_strip_16(png);
    png_set_gray_to_rgb(png);
    png_set_filler(png, 0xff, PNG_FILLER_AFTER);
    (void)png_set_interlace_handling(png);
    png_read_update_info(png, info);
    if (png_get_rowbytes(png, info) != (size_t)w * 4) png_error(png, "row size");
    rows = malloc(sizeof(png_bytep) * h);
    if (!rows) png_error(png, "memory");
    for (uint32_t y = 0; y < h; y++) rows[y] = out + (size_t)y * w * 4;
    png_read_image(png, rows);
    free(rows);
    png_destroy_read_struct(&png, &info, NULL);
    return 0;
}
