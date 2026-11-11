#import <AppKit/AppKit.h>
#import <QuickLookUI/QuickLookUI.h>
#import <QuickLookThumbnailing/QuickLookThumbnailing.h>
#include "../nabi_preview.h"

static NSError *failure(void) {
    return [NSError errorWithDomain:@"dev.nabi.NabiView.preview" code:1
        userInfo:@{NSLocalizedDescriptionKey: @"This image could not be previewed within the size and decoding limits."}];
}

static dispatch_queue_t decodeQueue(void) {
    static dispatch_queue_t queue;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ queue = dispatch_queue_create("dev.nabi.preview.decode", DISPATCH_QUEUE_SERIAL); });
    return queue;
}

static NSImage *decode(NSURL *url, uint32_t side) {
    if (!url.isFileURL) return nil;
    BOOL access = [url startAccessingSecurityScopedResource];
    NabiPreview result = {0};
    int status = nabi_preview_decode_path(url.fileSystemRepresentation, side, &result);
    if (access) [url stopAccessingSecurityScopedResource];
    if (status != 0) return nil;
    NSData *data = [NSData dataWithBytes:result.rgba length:result.length];
    CGDataProviderRef provider = CGDataProviderCreateWithCFData((__bridge CFDataRef)data);
    CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGImageRef image = CGImageCreate(result.width, result.height, 8, 32, result.width * 4,
        color, kCGImageAlphaLast | kCGBitmapByteOrderDefault, provider, NULL, true,
        kCGRenderingIntentDefault);
    NSImage *output = image ? [[NSImage alloc] initWithCGImage:image size:NSMakeSize(result.width, result.height)] : nil;
    if (image) CGImageRelease(image);
    CGColorSpaceRelease(color);
    CGDataProviderRelease(provider);
    nabi_preview_free(&result);
    return output;
}

#ifdef NABI_THUMBNAIL
@interface NabiThumbnailProvider : QLThumbnailProvider
@end
@implementation NabiThumbnailProvider
- (void)provideThumbnailForFileRequest:(QLFileThumbnailRequest *)request
                   completionHandler:(void (^)(QLThumbnailReply *, NSError *))handler {
    dispatch_async(decodeQueue(), ^{
        @autoreleasepool {
            CGFloat scale = MAX(1, request.scale);
            double requested = MAX(request.maximumSize.width, request.maximumSize.height) * scale;
            uint32_t side = (uint32_t)MAX(1, MIN(2048, ceil(requested)));
            NSImage *image = decode(request.fileURL, side);
            if (!image) { handler(nil, failure()); return; }
            CGSize size = CGSizeMake(image.size.width / scale, image.size.height / scale);
            CGFloat fit = MIN(request.maximumSize.width / size.width, request.maximumSize.height / size.height);
            size.width *= fit; size.height *= fit;
            QLThumbnailReply *reply = [QLThumbnailReply replyWithContextSize:size currentContextDrawingBlock:^BOOL{
                [image drawInRect:NSMakeRect(0, 0, size.width, size.height)
                        fromRect:NSZeroRect operation:NSCompositingOperationCopy fraction:1];
                return YES;
            }];
            handler(reply, nil);
        }
    });
}
@end
#else
@interface NabiPreviewController : NSViewController <QLPreviewingController>
@property NSImageView *imageView;
@property NSUInteger generation;
@end
@implementation NabiPreviewController
- (void)loadView {
    self.imageView = [[NSImageView alloc] initWithFrame:NSMakeRect(0, 0, 800, 600)];
    self.imageView.imageScaling = NSImageScaleProportionallyUpOrDown;
    self.imageView.imageAlignment = NSImageAlignCenter;
    self.view = self.imageView;
}
- (void)preparePreviewOfFileAtURL:(NSURL *)url completionHandler:(void (^)(NSError *))handler {
    NSUInteger generation = ++self.generation;
    dispatch_async(decodeQueue(), ^{
        @autoreleasepool {
            if (generation != self.generation) {
                dispatch_async(dispatch_get_main_queue(), ^{ handler([NSError errorWithDomain:NSCocoaErrorDomain code:NSUserCancelledError userInfo:nil]); });
                return;
            }
            NSImage *image = decode(url, 2048);
            dispatch_async(dispatch_get_main_queue(), ^{
                if (generation != self.generation) { handler([NSError errorWithDomain:NSCocoaErrorDomain code:NSUserCancelledError userInfo:nil]); return; }
                if (!image) { handler(failure()); return; }
                (void)self.view;
                self.imageView.image = image;
                self.preferredContentSize = image.size;
                handler(nil);
            });
        }
    });
}
@end
#endif

// The same entry point is supplied by Xcode for NSExtension principal objects.
extern int NSExtensionMain(int argc, const char *argv[]);
int main(int argc, const char *argv[]) { return NSExtensionMain(argc, argv); }
