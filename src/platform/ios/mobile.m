// API review 2026-11-08: https://developer.apple.com/documentation/uikit/uidocumentpickerviewcontroller
#import <UIKit/UIKit.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <CoreText/CoreText.h>
#import <ImageIO/ImageIO.h>
#include <fcntl.h>
#include <stdbool.h>

extern bool nabi_mobile_import_begin(void);
extern bool nabi_mobile_import_file(int fd, const char *name);
extern void nabi_mobile_import_error(const char *message);
extern void nabi_mobile_import_finish(void);

static UIWindow *nabi_window(void) {
    for (UIScene *scene in UIApplication.sharedApplication.connectedScenes) {
        if (![scene isKindOfClass:UIWindowScene.class]) continue;
        for (UIWindow *window in ((UIWindowScene *)scene).windows) if (window.isKeyWindow) return window;
    }
    // winit 0.30 uses UIApplicationDelegate without a scene manifest.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    for (UIWindow *window in UIApplication.sharedApplication.windows) if (window.isKeyWindow) return window;
#pragma clang diagnostic pop
    return nil;
}

const char *nabi_mobile_path(int kind) {
    static NSString *paths[3];
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        paths[0] = [NSSearchPathForDirectoriesInDomains(NSApplicationSupportDirectory, NSUserDomainMask, YES) firstObject];
        paths[1] = [NSSearchPathForDirectoriesInDomains(NSCachesDirectory, NSUserDomainMask, YES) firstObject];
        paths[2] = NSBundle.mainBundle.resourcePath;
    });
    return kind >= 0 && kind < 3 ? paths[kind].UTF8String : NULL;
}

const char *nabi_mobile_font(void) {
    static NSString *path;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        CTFontRef font = CTFontCreateWithName(CFSTR("AppleSDGothicNeo-Regular"), 14., NULL);
        if (font) {
            NSURL *url = CFBridgingRelease(CTFontCopyAttribute(font, kCTFontURLAttribute));
            path = url.path;
            CFRelease(font);
        }
    });
    return path.UTF8String;
}

void nabi_mobile_safe_area(float *out) {
    UIWindow *window = nabi_window();
    UIEdgeInsets insets = window.safeAreaInsets;
    CGFloat scale = window.screen.scale ?: 1.;
    out[0] = insets.top * scale; out[1] = insets.right * scale;
    out[2] = insets.bottom * scale; out[3] = insets.left * scale;
}

void nabi_mobile_keep_awake(bool enabled) {
    dispatch_async(dispatch_get_main_queue(), ^{
        UIApplication.sharedApplication.idleTimerDisabled = enabled && UIApplication.sharedApplication.applicationState == UIApplicationStateActive;
    });
}

@interface NabiDocumentPicker : NSObject <UIDocumentPickerDelegate>
@property(nonatomic) BOOL busy;
@end

@implementation NabiDocumentPicker
- (void)documentPickerWasCancelled:(UIDocumentPickerViewController *)controller {
    (void)controller; self.busy = NO;
}
- (void)documentPicker:(UIDocumentPickerViewController *)controller didPickDocumentsAtURLs:(NSArray<NSURL *> *)urls {
    (void)controller;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        BOOL ok = nabi_mobile_import_begin();
        for (NSURL *url in urls) {
            if (!ok) break;
            BOOL access = [url startAccessingSecurityScopedResource];
            NSFileCoordinator *coordinator = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
            NSError *error = nil;
            __block BOOL copied = NO;
            [coordinator coordinateReadingItemAtURL:url options:0 error:&error byAccessor:^(NSURL *readURL) {
                // Rust consumes and closes this read-only descriptor before the
                // security scope and coordinated access are released.
                int fd = open(readURL.fileSystemRepresentation, O_RDONLY | O_CLOEXEC);
                copied = nabi_mobile_import_file(fd, url.lastPathComponent.UTF8String);
            }];
            if (access) [url stopAccessingSecurityScopedResource];
            if (error) nabi_mobile_import_error(error.localizedDescription.UTF8String);
            ok = copied && error == nil;
        }
        nabi_mobile_import_finish();
        dispatch_async(dispatch_get_main_queue(), ^{ self.busy = NO; });
    });
}
@end

void nabi_mobile_pick(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        static NabiDocumentPicker *delegate;
        if (!delegate) delegate = [NabiDocumentPicker new];
        if (delegate.busy) return;
        UIViewController *presenter = nabi_window().rootViewController;
        while (presenter.presentedViewController) presenter = presenter.presentedViewController;
        if (!presenter) return;
        // Generic data also exposes AVIF/JXL/RAW extensions not registered with
        // the OS. The shared Rust import layer validates supported extensions.
        UIDocumentPickerViewController *picker = [[UIDocumentPickerViewController alloc]
            initForOpeningContentTypes:@[UTTypeImage, UTTypeData] asCopy:NO];
        picker.allowsMultipleSelection = YES;
        picker.delegate = delegate;
        delegate.busy = YES;
        [presenter presentViewController:picker animated:YES completion:nil];
    });
}

int nabi_decode_image(const char *path, unsigned char **pixels, unsigned int *width, unsigned int *height) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, NULL);
        if (!source) return -1;
        NSDictionary *properties = CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source, 0, NULL));
        uint64_t w = [properties[(__bridge NSString *)kCGImagePropertyPixelWidth] unsignedLongLongValue];
        uint64_t h = [properties[(__bridge NSString *)kCGImagePropertyPixelHeight] unsignedLongLongValue];
        if (!w || !h || w > 32768 || h > 32768 || w * h > 64000000) { CFRelease(source); return -2; }
        CGImageRef image = CGImageSourceCreateImageAtIndex(source, 0, NULL);
        CFRelease(source);
        if (!image) return -3;
        unsigned char *data = calloc(w * h, 4);
        CGColorSpaceRef color = CGColorSpaceCreateDeviceRGB();
        CGContextRef context = data ? CGBitmapContextCreate(data, w, h, 8, w * 4, color, kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big) : NULL;
        CGColorSpaceRelease(color);
        if (!context) { free(data); CGImageRelease(image); return -4; }
        CGContextDrawImage(context, CGRectMake(0, 0, w, h), image);
        CGContextRelease(context); CGImageRelease(image);
        // image::RgbaImage expects straight alpha.
        for (uint64_t i = 0; i < w * h; ++i) {
            unsigned a = data[i * 4 + 3];
            if (a && a < 255) for (int c = 0; c < 3; ++c) data[i * 4 + c] = MIN(255, (data[i * 4 + c] * 255 + a / 2) / a);
        }
        *pixels = data; *width = (unsigned)w; *height = (unsigned)h;
        return 0;
    }
}
void nabi_free_image(unsigned char *pixels) { free(pixels); }

void nabi_mobile_copy_text(const unsigned short *text, int length) {
    if (length < 0) return;
    NSString *value = [[NSString alloc] initWithCharacters:text length:(NSUInteger)length];
    dispatch_async(dispatch_get_main_queue(), ^{ UIPasteboard.generalPasteboard.string = value; });
}
