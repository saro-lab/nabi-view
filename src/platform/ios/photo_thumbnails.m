// PhotoKit's small, local, current-version preview path is independent of the
// original-resource loader and of the app's color-managed display decoder.
// API review (2026-11-16):
// https://developer.apple.com/documentation/photos/phimagemanager
// https://developer.apple.com/documentation/photos/phimagerequestoptionsversion/current
#import <Photos/Photos.h>
#import <UIKit/UIKit.h>
#import "photo_thumbnails.h"
#include <math.h>
#include <string.h>

// The callback owns this object, including after the synchronous C call ends.
// No callback captures rgba, width/height, live, or the caller's context.
@interface NabiPhotoThumbnailReply : NSObject {
@public
    NSLock *lock;
    dispatch_semaphore_t signal;
    BOOL active;
    BOOL completed;
    UIImage *image;
}
@end
@implementation NabiPhotoThumbnailReply
- (instancetype)init {
    if ((self = [super init])) {
        lock = [NSLock new];
        signal = dispatch_semaphore_create(0);
        active = YES;
    }
    return self;
}
@end

static NSLock *nabi_thumbnail_gate(void) {
    static NSLock *gate;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ gate = [NSLock new]; });
    return gate;
}

static BOOL nabi_thumbnail_readable(void) {
    PHAuthorizationStatus status = [PHPhotoLibrary authorizationStatusForAccessLevel:PHAccessLevelReadWrite];
    return status == PHAuthorizationStatusAuthorized || status == PHAuthorizationStatusLimited;
}

// Match the same FullSizePhoto-before-Photo selection and exact snapshot name
// used by photo_snapshot.m and photos.m, including any edited resource change.
static PHAsset *nabi_thumbnail_asset(NSString *token, NSString *name, int type,
                                    double modified, int *failure) {
    if (!nabi_thumbnail_readable()) { *failure = NABI_PHOTO_THUMBNAIL_UNAVAILABLE; return nil; }
    PHAsset *asset = [PHAsset fetchAssetsWithLocalIdentifiers:@[token] options:nil].firstObject;
    if (!asset || asset.mediaType != PHAssetMediaTypeImage) {
        *failure = NABI_PHOTO_THUMBNAIL_UNAVAILABLE; return nil;
    }
    PHAssetResource *selected = nil;
    for (PHAssetResource *resource in [PHAssetResource assetResourcesForAsset:asset]) {
        if (resource.type == PHAssetResourceTypeFullSizePhoto) { selected = resource; break; }
        if (resource.type == PHAssetResourceTypePhoto) selected = resource;
    }
    NSString *key = [asset.localIdentifier componentsSeparatedByString:@"/"].firstObject;
    NSString *currentName = selected ? [NSString stringWithFormat:@"%@-%@", key, selected.originalFilename] : nil;
    if (!selected || selected.type != type || ![currentName isEqualToString:name]
            || !asset.modificationDate || asset.modificationDate.timeIntervalSince1970 != modified) {
        *failure = NABI_PHOTO_THUMBNAIL_CHANGED; return nil;
    }
    return asset;
}

static BOOL nabi_thumbnail_rgba(UIImage *image, uint32_t edge, uint8_t *rgba,
                                uint32_t *width, uint32_t *height) {
    CGImageRef cg = image.CGImage;
    if (!cg) return NO;
    size_t w = CGImageGetWidth(cg), h = CGImageGetHeight(cg);
    // Photos may serve a slightly larger cached representation. Accept only a
    // still-small input (at most 512² pixels) and always bound our output by
    // edge. Never render an unexpectedly full-size UIImage into a second bitmap.
    if (!w || !h || w > 512 || h > 512) return NO;
    switch (image.imageOrientation) {
        case UIImageOrientationLeft: case UIImageOrientationRight:
        case UIImageOrientationLeftMirrored: case UIImageOrientationRightMirrored: {
            size_t swap = w; w = h; h = swap; break;
        }
        default: break;
    }
    double scale = fmin(1., (double)edge / (double)MAX(w, h));
    w = MAX(1, (size_t)floor((double)w * scale));
    h = MAX(1, (size_t)floor((double)h * scale));
    memset(rgba, 0, w * h * 4);
    CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    if (!color) return NO;
    CGContextRef bitmap = CGBitmapContextCreate(rgba, w, h, 8, w * 4, color,
        kCGBitmapByteOrder32Big | kCGImageAlphaPremultipliedLast);
    CGColorSpaceRelease(color);
    if (!bitmap) return NO;
    // UIKit drawing applies UIImage.imageOrientation. The flipped CGContext
    // makes the first byte row the top of the upright output.
    CGContextTranslateCTM(bitmap, 0, h);
    CGContextScaleCTM(bitmap, 1, -1);
    CGContextSetInterpolationQuality(bitmap, kCGInterpolationHigh);
    UIGraphicsPushContext(bitmap);
    [image drawInRect:CGRectMake(0, 0, w, h) blendMode:kCGBlendModeCopy alpha:1.];
    UIGraphicsPopContext();
    CGContextRelease(bitmap);
    for (size_t pixel = 0; pixel < w * h; pixel++) {
        uint8_t *p = rgba + pixel * 4;
        unsigned a = p[3];
        for (size_t channel = 0; channel < 3; channel++) {
            p[channel] = a ? (uint8_t)MIN(255u, ((unsigned)p[channel] * 255u + a / 2u) / a) : 0;
        }
    }
    *width = (uint32_t)w;
    *height = (uint32_t)h;
    return YES;
}

static int nabi_thumbnail_request(NSString *token, NSString *name, int resourceType,
                                  double modified, uint32_t edge, uint8_t *rgba,
                                  uint32_t *width, uint32_t *height,
                                  bool (*live)(void *), void *context) {
    if (!live(context)) return NABI_PHOTO_THUMBNAIL_CANCELLED;
    NSTimeInterval deadline = NSProcessInfo.processInfo.systemUptime + 2.;
    int failure = NABI_PHOTO_THUMBNAIL_UNAVAILABLE;
    PHAsset *asset = nabi_thumbnail_asset(token, name, resourceType, modified, &failure);
    if (!asset) return failure;
    if (!live(context)) return NABI_PHOTO_THUMBNAIL_CANCELLED;
    if (NSProcessInfo.processInfo.systemUptime >= deadline) return NABI_PHOTO_THUMBNAIL_TIMEOUT;
    PHImageRequestOptions *options = [PHImageRequestOptions new];
    options.synchronous = NO; // Native async request keeps cancellation possible.
    options.networkAccessAllowed = NO;
    options.version = PHImageRequestOptionsVersionCurrent;
    options.deliveryMode = PHImageRequestOptionsDeliveryModeHighQualityFormat;
    options.resizeMode = PHImageRequestOptionsResizeModeExact;
    if (@available(iOS 17, *)) { options.preferHDR = NO; options.targetHDRHeadroom = 1.; }
    NabiPhotoThumbnailReply *reply = [NabiPhotoThumbnailReply new];
    PHImageManager *manager = PHImageManager.defaultManager;
    PHImageRequestID request = [manager requestImageForAsset:asset targetSize:CGSizeMake(edge, edge)
        contentMode:PHImageContentModeAspectFit options:options resultHandler:^(UIImage *result, NSDictionary *info) {
            @autoreleasepool {
                [reply->lock lock];
                if (reply->active && !reply->completed) {
                    // HighQualityFormat delivers once. A degraded/cloud/error
                    // result is a miss, never a reason to fetch an original.
                    BOOL usable = result && ![info[PHImageCancelledKey] boolValue]
                        && !info[PHImageErrorKey] && ![info[PHImageResultIsDegradedKey] boolValue];
                    reply->image = usable ? result : nil;
                    reply->completed = YES;
                    dispatch_semaphore_signal(reply->signal);
                }
                [reply->lock unlock];
            }
        }];
    UIImage *image = nil;
    int result = NABI_PHOTO_THUMBNAIL_UNAVAILABLE;
    for (;;) {
        if (!live(context)) { result = NABI_PHOTO_THUMBNAIL_CANCELLED; break; }
        if (NSProcessInfo.processInfo.systemUptime >= deadline) { result = NABI_PHOTO_THUMBNAIL_TIMEOUT; break; }
        [reply->lock lock];
        BOOL done = reply->completed;
        if (done) image = reply->image;
        [reply->lock unlock];
        if (done) break;
        dispatch_semaphore_wait(reply->signal, dispatch_time(DISPATCH_TIME_NOW, 25 * NSEC_PER_MSEC));
    }
    [reply->lock lock];
    reply->active = NO;
    reply->image = nil;
    [reply->lock unlock];
    // Also cancel on completion: an unexpected subsequent callback is harmless
    // and cannot retain caller storage or start further app-side conversion.
    if (request != PHInvalidImageRequestID) [manager cancelImageRequest:request];
    if (!image) return result;
    uint32_t w = 0, h = 0;
    if (!nabi_thumbnail_rgba(image, edge, rgba, &w, &h)) return NABI_PHOTO_THUMBNAIL_UNAVAILABLE;
    if (!live(context)) return NABI_PHOTO_THUMBNAIL_CANCELLED;
    if (NSProcessInfo.processInfo.systemUptime >= deadline) return NABI_PHOTO_THUMBNAIL_TIMEOUT;
    // Refetch after rendering to catch deletion, permission revocation or edits
    // while PhotoKit was producing the result; the original PHAsset is a snapshot.
    if (!nabi_thumbnail_asset(token, name, resourceType, modified, &failure)) return failure;
    if (!live(context)) return NABI_PHOTO_THUMBNAIL_CANCELLED;
    if (NSProcessInfo.processInfo.systemUptime >= deadline) return NABI_PHOTO_THUMBNAIL_TIMEOUT;
    *width = w; *height = h;
    return NABI_PHOTO_THUMBNAIL_OK;
}

int nabi_photo_thumbnail(const char *asset_token, const char *resource_name,
                         int resource_type, double content_modified,
                         uint32_t max_edge, uint8_t *rgba, size_t capacity,
                         uint32_t *width, uint32_t *height,
                         bool (*live)(void *), void *context) {
    if (width) *width = 0;
    if (height) *height = 0;
    if (!asset_token || !resource_name || !rgba || !width || !height || !live
            || !max_edge || max_edge > 256 || capacity < (size_t)max_edge * max_edge * 4
            || !isfinite(content_modified) || content_modified <= 0 || NSThread.isMainThread) {
        return NABI_PHOTO_THUMBNAIL_INVALID;
    }
    @autoreleasepool {
        NSString *token = [NSString stringWithUTF8String:asset_token];
        NSString *name = [NSString stringWithUTF8String:resource_name];
        if (!token.length || !name.length) return NABI_PHOTO_THUMBNAIL_INVALID;
        NSLock *gate = nabi_thumbnail_gate();
        if (![gate tryLock]) return NABI_PHOTO_THUMBNAIL_BUSY;
        int result;
        @try {
            result = nabi_thumbnail_request(token, name, resource_type, content_modified,
                max_edge, rgba, width, height, live, context);
        } @finally {
            [gate unlock];
        }
        return result;
    }
}
