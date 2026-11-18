// Small PhotoKit thumbnails only. Never use these pixels as a display/original
// cache entry: Photos renders current adjustments using its own SDR pipeline.
#ifndef NABI_PHOTO_THUMBNAILS_H
#define NABI_PHOTO_THUMBNAILS_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

enum {
    NABI_PHOTO_THUMBNAIL_OK = 0,
    NABI_PHOTO_THUMBNAIL_CANCELLED = -1,
    NABI_PHOTO_THUMBNAIL_UNAVAILABLE = 1,
    NABI_PHOTO_THUMBNAIL_CHANGED = 2,
    NABI_PHOTO_THUMBNAIL_INVALID = 3,
    NABI_PHOTO_THUMBNAIL_TIMEOUT = 4,
    NABI_PHOTO_THUMBNAIL_BUSY = 5,
};

// Call from a background worker. One process-wide request is admitted at once;
// a competing call returns BUSY without waiting. No access prompt or download.
// resource_name is the snapshot name: <asset identifier before '/'>-<original
// resource filename>. resource_type and content_modified must come from that
// same snapshot; content_modified is modificationDate, never creationDate.
//
// max_edge is 1..256; capacity must be >= max_edge * max_edge * 4. On success,
// rgba holds width * height * 4 tightly packed, top-down, straight-alpha sRGB
// bytes in RGBA order, with UIImage orientation applied. Width/height are zero
// on failure and the caller must discard the buffer. Output is thumbnail-sized.
//
// live/context are borrowed for this call only, polled by its calling thread
// every 25 ms while waiting (at most 2 seconds), and never used by callbacks.
// A false live result cancels the PhotoKit request. Late callbacks own only
// Objective-C state, never caller pointers. OS metadata/request entry calls
// themselves cannot be forcibly interrupted; the wait is explicitly bounded.
int nabi_photo_thumbnail(const char *asset_token, const char *resource_name,
                         int resource_type, double content_modified,
                         uint32_t max_edge, uint8_t *rgba, size_t capacity,
                         uint32_t *width, uint32_t *height,
                         bool (*live)(void *), void *context);
#endif
