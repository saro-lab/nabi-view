// API review 2026-11-08: https://developer.apple.com/documentation/uikit/uidocumentpickerviewcontroller
#import <UIKit/UIKit.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <CoreText/CoreText.h>
#import <ImageIO/ImageIO.h>
#import <objc/runtime.h>
#include <fcntl.h>
#include <stdbool.h>
#include <os/proc.h>
#include <mach/mach.h>

extern bool nabi_mobile_import_begin(void);
extern bool nabi_mobile_import_begin_folder(void);
extern void nabi_mobile_settings_import(int fd);
extern void nabi_mobile_export_result(bool saved);
extern bool nabi_mobile_import_file(int fd, const char *name);
extern void nabi_mobile_import_error(const char *message);
extern void nabi_mobile_import_finish(void);
extern void nabi_mobile_import_source(const char *id, const char *name);

// Available memory is constrained by both system RAM and the app's jetsam
// allocation ceiling. Unlike desktops, total is the effective app ceiling.
bool nabi_mobile_memory(uint64_t *total, uint64_t *available) {
    @autoreleasepool {
        uint64_t headroom = os_proc_available_memory();
        task_vm_info_data_t task = {0};
        mach_msg_type_number_t count = TASK_VM_INFO_COUNT;
        if (task_info(mach_task_self(), TASK_VM_INFO, (task_info_t)&task, &count) != KERN_SUCCESS) return false;
        uint64_t physical = NSProcessInfo.processInfo.physicalMemory;
        if (!physical || UINT64_MAX - headroom < task.phys_footprint) return false;
        *total = MIN(physical, headroom + task.phys_footprint);
        *available = MIN(headroom, *total);
        mach_port_t host = mach_host_self();
        vm_statistics_data_t vm = {0};
        mach_msg_type_number_t vm_count = HOST_VM_INFO_COUNT;
        vm_size_t page = 0;
        if (host && host_page_size(host, &page) == KERN_SUCCESS &&
            host_statistics(host, HOST_VM_INFO, (host_info_t)&vm, &vm_count) == KERN_SUCCESS) {
            uint64_t free_pages = (uint64_t)vm.free_count + vm.inactive_count;
            if (page && free_pages <= UINT64_MAX / page) *available = MIN(*available, free_pages * page);
        }
        if (host) mach_port_deallocate(mach_task_self(), host);
        return *total > 0;
    }
}

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

// winit owns the root view controller. Its class answers orientation and
// home-indicator queries from these values instead of winit's defaults.
static UIInterfaceOrientationMask nabi_orientations = UIInterfaceOrientationMaskAll;
static BOOL nabi_immersive;
static UIInterfaceOrientationMask nabi_supported_orientations(id self, SEL command) {
    (void)self; (void)command; return nabi_orientations;
}
static BOOL nabi_home_indicator_hidden(id self, SEL command) {
    (void)self; (void)command; return nabi_immersive;
}
static void nabi_apply_orientation(UIViewController *controller, UIWindow *window) {
    if (@available(iOS 16.0, *)) {
        [controller setNeedsUpdateOfSupportedInterfaceOrientations];
        if (nabi_orientations != UIInterfaceOrientationMaskAll && window.windowScene) {
            UIWindowSceneGeometryPreferencesIOS *preferences =
                [[UIWindowSceneGeometryPreferencesIOS alloc] initWithInterfaceOrientations:nabi_orientations];
            [window.windowScene requestGeometryUpdateWithPreferences:preferences errorHandler:nil];
        }
    } else {
        [UIViewController attemptRotationToDeviceOrientation];
    }
}
static UIViewController *nabi_controller(UIWindow *window) {
    static Class installed;
    UIViewController *controller = window.rootViewController;
    if (!controller) return nil;
    Class cls = object_getClass(controller);
    if (cls != installed) {
        class_replaceMethod(cls, @selector(supportedInterfaceOrientations), (IMP)nabi_supported_orientations, "Q@:");
        class_replaceMethod(cls, @selector(prefersHomeIndicatorAutoHidden), (IMP)nabi_home_indicator_hidden, "B@:");
        installed = cls;
        // A lock requested before the window existed applies now.
        nabi_apply_orientation(controller, window);
    }
    return controller;
}

void nabi_mobile_set_orientation(int mode) {
    dispatch_async(dispatch_get_main_queue(), ^{
        nabi_orientations = mode == 1 ? (UIInterfaceOrientationMaskPortrait | UIInterfaceOrientationMaskPortraitUpsideDown)
            : mode == 2 ? UIInterfaceOrientationMaskLandscape : UIInterfaceOrientationMaskAll;
        UIWindow *window = nabi_window();
        UIViewController *controller = nabi_controller(window);
        if (controller) nabi_apply_orientation(controller, window);
    });
}

void nabi_mobile_set_immersive(bool enabled) {
    dispatch_async(dispatch_get_main_queue(), ^{
        nabi_immersive = enabled;
        // Info.plist keeps application-based status bar appearance.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        [UIApplication.sharedApplication setStatusBarHidden:enabled withAnimation:UIStatusBarAnimationFade];
#pragma clang diagnostic pop
        UIViewController *controller = nabi_controller(nabi_window());
        [controller setNeedsUpdateOfHomeIndicatorAutoHidden];
    });
}

static UIViewController *nabi_presenter(void) {
    UIViewController *presenter = nabi_window().rootViewController;
    while (presenter.presentedViewController) presenter = presenter.presentedViewController;
    return presenter;
}

void nabi_mobile_share(const unsigned short *text, int length) {
    if (length < 0) return;
    NSString *path = [[NSString alloc] initWithCharacters:text length:(NSUInteger)length];
    dispatch_async(dispatch_get_main_queue(), ^{
        UIViewController *presenter = nabi_presenter();
        if (!presenter) return;
        UIActivityViewController *sheet = [[UIActivityViewController alloc]
            initWithActivityItems:@[[NSURL fileURLWithPath:path]] applicationActivities:nil];
        // iPad presents the sheet as a popover anchored above the bottom bar.
        CGRect bounds = presenter.view.bounds;
        sheet.popoverPresentationController.sourceView = presenter.view;
        sheet.popoverPresentationController.sourceRect = CGRectMake(CGRectGetMidX(bounds), CGRectGetMaxY(bounds) - 48., 1., 1.);
        [presenter presentViewController:sheet animated:YES completion:nil];
    });
}

void nabi_mobile_safe_area(float *out) {
    UIWindow *window = nabi_window();
    nabi_controller(window);
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
@property(nonatomic) BOOL folder;
@end

static NSString *nabi_grants_path(void) {
    NSString *directory = [NSString stringWithUTF8String:nabi_mobile_path(0)];
    [NSFileManager.defaultManager createDirectoryAtPath:directory withIntermediateDirectories:YES attributes:nil error:nil];
    return [directory stringByAppendingPathComponent:@"document-grants.plist"];
}

static NSMutableDictionary *nabi_grants(void) {
    NSData *data = [NSData dataWithContentsOfFile:nabi_grants_path() options:NSDataReadingMappedIfSafe error:nil];
    if (data.length > 8 * 1024 * 1024) return [NSMutableDictionary new];
    id values = data ? [NSPropertyListSerialization propertyListWithData:data options:NSPropertyListMutableContainers format:nil error:nil] : nil;
    return [values isKindOfClass:NSMutableDictionary.class] ? values : [NSMutableDictionary new];
}

static BOOL nabi_write_grants(NSDictionary *grants) {
    NSData *data = [NSPropertyListSerialization dataWithPropertyList:grants format:NSPropertyListBinaryFormat_v1_0 options:0 error:nil];
    NSString *path = nabi_grants_path();
    BOOL saved = data && data.length <= 8 * 1024 * 1024 && [data writeToFile:path options:NSDataWritingAtomic error:nil];
    if (saved) [[NSURL fileURLWithPath:path] setResourceValue:@YES forKey:NSURLIsExcludedFromBackupKey error:nil];
    return saved;
}

// iOS picker URLs carry implicit security scope. The macOS-only
// NSURLBookmarkCreationWithSecurityScope option must not be used here.
static NSString *nabi_save_source(NSArray<NSURL *> *urls, BOOL folder) {
    if (!urls.count || urls.count > 128) return nil;
    NSMutableArray *bookmarks = [NSMutableArray new], *locations = [NSMutableArray new];
    for (NSURL *url in urls) {
        BOOL access = [url startAccessingSecurityScopedResource];
        NSData *bookmark = [url bookmarkDataWithOptions:NSURLBookmarkCreationMinimalBookmark
            includingResourceValuesForKeys:nil relativeToURL:nil error:nil];
        if (access) [url stopAccessingSecurityScopedResource];
        if (!bookmark) return nil;
        [bookmarks addObject:bookmark]; [locations addObject:url.absoluteString];
    }
    NSMutableDictionary *grants = nabi_grants();
    NSString *identifier = nil;
    for (NSString *key in grants) {
        NSDictionary *record = grants[key];
        if ([record isKindOfClass:NSDictionary.class] && [record[@"folder"] boolValue] == folder && [record[@"locations"] isEqual:locations]) {
            identifier = key; break;
        }
    }
    if (!identifier) identifier = NSUUID.UUID.UUIDString;
    grants[identifier] = @{@"bookmarks":bookmarks, @"locations":locations, @"folder":@(folder), @"name":urls.firstObject.lastPathComponent ?: @"Selected documents"};
    return nabi_write_grants(grants) ? identifier : nil;
}

static void nabi_report_source(NSString *identifier, NSString *fallback) {
    if (!identifier) return;
    NSDictionary *record = nabi_grants()[identifier];
    if (![record isKindOfClass:NSDictionary.class]) return;
    NSString *name = [record[@"name"] isKindOfClass:NSString.class] ? record[@"name"] : fallback;
    nabi_mobile_import_source(identifier.UTF8String, name.UTF8String);
}

static void nabi_import_folder(NSURL *url, NSString *identifier) {
    BOOL access = [url startAccessingSecurityScopedResource];
    __block NSError *listError = nil;
    NSError *coordinationError = nil;
    __block NSArray<NSURL *> *items = nil;
    NSFileCoordinator *listing = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
    [listing coordinateReadingItemAtURL:url options:0 error:&coordinationError byAccessor:^(NSURL *readURL) {
        items = [NSFileManager.defaultManager contentsOfDirectoryAtURL:readURL
            includingPropertiesForKeys:@[NSURLIsRegularFileKey] options:NSDirectoryEnumerationSkipsHiddenFiles error:&listError];
    }];
    if (!nabi_mobile_import_begin_folder()) items = @[];
    nabi_report_source(identifier, url.lastPathComponent);
    if (listError || coordinationError) nabi_mobile_import_error("Folder access is unavailable. Select the folder again.");
    for (NSURL *item in items) {
        NSNumber *regular = nil;
        [item getResourceValue:&regular forKey:NSURLIsRegularFileKey error:nil];
        if (!regular.boolValue) continue;
        // Rust consumes and closes the descriptor and skips unsupported files.
        __block BOOL more = YES;
        NSFileCoordinator *coordinator = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
        [coordinator coordinateReadingItemAtURL:item options:0 error:nil byAccessor:^(NSURL *readURL) {
            int fd = open(readURL.fileSystemRepresentation, O_RDONLY | O_CLOEXEC);
            if (fd >= 0) more = nabi_mobile_import_file(fd, item.lastPathComponent.UTF8String);
        }];
        if (!more) break;
    }
    if (access) [url stopAccessingSecurityScopedResource];
    nabi_mobile_import_finish();
}

static void nabi_import_documents(NSArray<NSURL *> *urls, NSString *identifier) {
    BOOL ok = nabi_mobile_import_begin();
    nabi_report_source(identifier, urls.firstObject.lastPathComponent ?: @"Selected documents");
    if (urls.count > 128) { nabi_mobile_import_error("Select at most 128 documents."); ok = NO; }
    for (NSURL *url in urls) {
        if (!ok) break;
        BOOL access = [url startAccessingSecurityScopedResource];
        NSFileCoordinator *coordinator = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
        NSError *error = nil;
        __block BOOL copied = NO;
        [coordinator coordinateReadingItemAtURL:url options:0 error:&error byAccessor:^(NSURL *readURL) {
            int fd = open(readURL.fileSystemRepresentation, O_RDONLY | O_CLOEXEC);
            copied = nabi_mobile_import_file(fd, url.lastPathComponent.UTF8String);
        }];
        if (access) [url stopAccessingSecurityScopedResource];
        if (error) nabi_mobile_import_error("Document access is unavailable. Select the document again.");
        ok = copied && error == nil;
    }
    nabi_mobile_import_finish();
}

@implementation NabiDocumentPicker
- (void)documentPickerWasCancelled:(UIDocumentPickerViewController *)controller {
    (void)controller; self.busy = NO;
}
- (void)documentPicker:(UIDocumentPickerViewController *)controller didPickDocumentsAtURLs:(NSArray<NSURL *> *)urls {
    (void)controller;
    if (self.folder) {
        NSURL *folder = urls.firstObject;
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
            if (folder) nabi_import_folder(folder, nabi_save_source(@[folder], YES));
            dispatch_async(dispatch_get_main_queue(), ^{ self.busy = NO; });
        });
        return;
    }
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        nabi_import_documents(urls, nabi_save_source(urls, NO));
        dispatch_async(dispatch_get_main_queue(), ^{ self.busy = NO; });
    });
}
@end

static NabiDocumentPicker *nabi_document_picker(void) {
    static NabiDocumentPicker *delegate;
    if (!delegate) delegate = [NabiDocumentPicker new];
    return delegate;
}

bool nabi_mobile_reopen(const unsigned short *text, int length) {
    if (length < 8 || length > 64) return false;
    NSString *identifier = [[NSString alloc] initWithCharacters:text length:(NSUInteger)length];
    __block BOOL accepted = NO;
    void (^start)(void) = ^{
        NabiDocumentPicker *delegate = nabi_document_picker();
        if (delegate.busy) return;
        delegate.busy = YES; accepted = YES;
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
            @autoreleasepool {
                NSMutableDictionary *grants = nabi_grants();
                NSDictionary *record = grants[identifier];
                NSArray *bookmarks = [record isKindOfClass:NSDictionary.class] ? record[@"bookmarks"] : nil;
                NSMutableArray<NSURL *> *urls = [NSMutableArray new];
                NSMutableArray *refreshed = [NSMutableArray new];
                if ([bookmarks isKindOfClass:NSArray.class] && bookmarks.count <= 128) {
                    for (id bookmark in bookmarks) {
                        if (![bookmark isKindOfClass:NSData.class]) break;
                        BOOL stale = NO;
                        NSURL *url = [NSURL URLByResolvingBookmarkData:bookmark options:NSURLBookmarkResolutionWithoutUI
                            relativeToURL:nil bookmarkDataIsStale:&stale error:nil];
                        if (!url) break;
                        NSData *replacement = bookmark;
                        if (stale) {
                            BOOL access = [url startAccessingSecurityScopedResource];
                            replacement = [url bookmarkDataWithOptions:NSURLBookmarkCreationMinimalBookmark
                                includingResourceValuesForKeys:nil relativeToURL:nil error:nil];
                            if (access) [url stopAccessingSecurityScopedResource];
                        }
                        if (!replacement) break;
                        [urls addObject:url]; [refreshed addObject:replacement];
                    }
                }
                if (urls.count && urls.count == bookmarks.count) {
                    NSMutableDictionary *updated = [record mutableCopy];
                    updated[@"bookmarks"] = refreshed;
                    updated[@"locations"] = [urls valueForKey:@"absoluteString"];
                    grants[identifier] = updated;
                    nabi_write_grants(grants);
                    if ([record[@"folder"] boolValue]) nabi_import_folder(urls.firstObject, identifier);
                    else nabi_import_documents(urls, identifier);
                } else {
                    nabi_mobile_import_begin();
                    nabi_mobile_import_error("Saved document access is unavailable. Select the source again.");
                    nabi_mobile_import_finish();
                }
                dispatch_async(dispatch_get_main_queue(), ^{ delegate.busy = NO; });
            }
        });
    };
    if (NSThread.isMainThread) start(); else dispatch_sync(dispatch_get_main_queue(), start);
    return accepted;
}

static void nabi_pick_documents(BOOL folder) {
    dispatch_async(dispatch_get_main_queue(), ^{
        NabiDocumentPicker *delegate = nabi_document_picker();
        if (delegate.busy) return;
        UIViewController *presenter = nabi_presenter();
        if (!presenter) return;
        // Generic data also exposes AVIF/JXL/RAW extensions not registered with
        // the OS. The shared Rust import layer validates supported extensions.
        UIDocumentPickerViewController *picker = [[UIDocumentPickerViewController alloc]
            initForOpeningContentTypes:folder ? @[UTTypeFolder] : @[UTTypeImage, UTTypeData] asCopy:NO];
        picker.allowsMultipleSelection = !folder;
        picker.delegate = delegate;
        delegate.folder = folder;
        delegate.busy = YES;
        [presenter presentViewController:picker animated:YES completion:nil];
    });
}

void nabi_mobile_pick(void) { nabi_pick_documents(NO); }
void nabi_mobile_pick_folder(void) { nabi_pick_documents(YES); }

@interface NabiSettingsPicker : NSObject <UIDocumentPickerDelegate>
@property(nonatomic) BOOL exporting;
@end

@implementation NabiSettingsPicker
- (void)documentPickerWasCancelled:(UIDocumentPickerViewController *)controller { (void)controller; }
- (void)documentPicker:(UIDocumentPickerViewController *)controller didPickDocumentsAtURLs:(NSArray<NSURL *> *)urls {
    (void)controller;
    if (self.exporting) return;
    NSURL *url = urls.firstObject;
    if (!url) return;
    // Opened as a copy, so no security scope is needed. Rust closes the descriptor.
    nabi_mobile_settings_import(open(url.fileSystemRepresentation, O_RDONLY | O_CLOEXEC));
}
@end

static void nabi_present_settings_picker(UIDocumentPickerViewController *picker, BOOL exporting) {
    static NabiSettingsPicker *delegate;
    if (!delegate) delegate = [NabiSettingsPicker new];
    UIViewController *presenter = nabi_presenter();
    if (!presenter) {
        if (exporting) nabi_mobile_export_result(false);
        return;
    }
    delegate.exporting = exporting;
    picker.delegate = delegate;
    [presenter presentViewController:picker animated:YES completion:nil];
}

void nabi_mobile_export_settings(const unsigned short *text, int length) {
    if (length < 0) return;
    NSString *path = [[NSString alloc] initWithCharacters:text length:(NSUInteger)length];
    dispatch_async(dispatch_get_main_queue(), ^{
        UIDocumentPickerViewController *picker = [[UIDocumentPickerViewController alloc]
            initForExportingURLs:@[[NSURL fileURLWithPath:path]] asCopy:YES];
        nabi_present_settings_picker(picker, YES);
    });
}

void nabi_mobile_import_settings(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        UIDocumentPickerViewController *picker = [[UIDocumentPickerViewController alloc]
            initForOpeningContentTypes:@[UTTypeJSON, UTTypeData] asCopy:YES];
        nabi_present_settings_picker(picker, NO);
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
