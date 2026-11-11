#ifndef NABI_PREVIEW_H
#define NABI_PREVIEW_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct { uint32_t width, height; size_t length; uint8_t *rgba; } NabiPreview;
int32_t nabi_preview_decode_path(const char *utf8_path, uint32_t max_side, NabiPreview *output);
int32_t nabi_preview_decode_bytes(const uint8_t *bytes, size_t length, const char *extension, uint32_t max_side, NabiPreview *output);
void nabi_preview_free(NabiPreview *output);
#ifdef __cplusplus
}
#endif
#endif
