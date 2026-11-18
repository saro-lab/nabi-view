// API review 2026-11-08: https://developer.apple.com/documentation/uikit/uidocumentpickerviewcontroller
#import <UIKit/UIKit.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <CoreText/CoreText.h>
#import <ImageIO/ImageIO.h>
#import <PhotosUI/PhotosUI.h>
#import <objc/runtime.h>
#include <fcntl.h>
#include <errno.h>
#include <math.h>
#include <stdbool.h>
#include <os/proc.h>
#include <mach/mach.h>
#include <unistd.h>
#include <sys/stat.h>

extern bool nabi_mobile_import_begin(void);
extern bool nabi_mobile_import_begin_folder(void);
extern void nabi_mobile_import_pending(void);
extern void nabi_mobile_settings_import(int fd);
extern void nabi_mobile_export_result(bool saved);
extern bool nabi_mobile_import_file(int fd, const char *name);
extern bool nabi_mobile_import_deferred(const char *name, const char *token, uint64_t bytes, double modified,
                                        int resource_type, double content_modified);
extern void nabi_mobile_import_error(const char *message);
extern void nabi_mobile_import_finish(void);
extern void nabi_mobile_import_source(const char *id, const char *name);
extern void nabi_mobile_window_mode_changed(int state);
extern void nabi_mobile_access_changed(void);
extern void nabi_mobile_access_resumed(void);
extern void nabi_mobile_access_action_result(int action, bool success);
extern void nabi_mobile_photo_error(int reason);
extern size_t nabi_mobile_text(const char *key, unsigned short *output, size_t capacity);
extern int nabi_mobile_photo_status(void);
extern bool nabi_photos_is_collection(NSString *identifier);
extern void nabi_photos_open(NSString *identifier, void (^completion)(void));
extern int nabi_photos_read(NSString *token, int fd, uint64_t limit, bool (*live)(void *), void *context);

// iOS 27 requires scene-based lifecycle adoption. UIKit creates the single
// window scene from Info.plist; winit attaches its own window and root view
// controller when the application becomes active. Keep ownership in winit.
@interface NabiSceneDelegate : UIResponder <UIWindowSceneDelegate>
@end

@implementation NabiSceneDelegate
- (void)sceneDidBecomeActive:(UIScene *)scene {
    (void)scene;
    nabi_mobile_access_resumed();
}
@end

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
    // Retain the fallback for windows that have not yet joined a scene.
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
// The OS-selected orientation before the viewer overrides it. In particular,
// Control Center's portrait lock need not generate a new device-rotation event
// when the user releases the viewer's landscape override while holding still.
static UIInterfaceOrientation nabi_system_orientation = UIInterfaceOrientationUnknown;
static __weak UIWindowScene *nabi_orientation_scene;
static BOOL nabi_immersive;
static int nabi_window_mode = -2;
static void nabi_apply_orientation(UIViewController *controller, UIWindow *window,
                                   UIInterfaceOrientationMask restoreOrientation);

// Compare the whole window with its own display in one coordinate space. Safe
// areas, device idiom, display count, and orientation request results do not
// determine whether this window is a standalone or a windowed presentation.
static int nabi_detect_window_mode(UIWindow *window) {
    UIScreen *screen = window.screen;
    if (!window || !screen || !window.rootViewController) return -1;
    CGRect frame = [window convertRect:window.bounds toCoordinateSpace:screen.coordinateSpace];
    CGRect display = screen.coordinateSpace.bounds;
    if (!isfinite(frame.origin.x) || !isfinite(frame.origin.y) ||
        !isfinite(frame.size.width) || !isfinite(frame.size.height) ||
        !isfinite(display.origin.x) || !isfinite(display.origin.y) ||
        !isfinite(display.size.width) || !isfinite(display.size.height) ||
        CGRectIsEmpty(frame) || CGRectIsEmpty(display)) return -1;
    CGFloat tolerance = 1. / MAX(screen.scale, 1.);
    BOOL fillsDisplay = fabs(CGRectGetMinX(frame) - CGRectGetMinX(display)) <= tolerance &&
        fabs(CGRectGetMinY(frame) - CGRectGetMinY(display)) <= tolerance &&
        fabs(CGRectGetMaxX(frame) - CGRectGetMaxX(display)) <= tolerance &&
        fabs(CGRectGetMaxY(frame) - CGRectGetMaxY(display)) <= tolerance;
    return fillsDisplay ? 0 : 1;
}

static void nabi_publish_window_mode(UIWindow *window) {
    int state = nabi_detect_window_mode(window);
    if (state == nabi_window_mode) return;
    nabi_window_mode = state;
    if (state == 1 && nabi_orientations != UIInterfaceOrientationMaskAll) {
        // Release a previous lock immediately; Rust retains the user's choice.
        nabi_orientations = UIInterfaceOrientationMaskAll;
        if (window.rootViewController) nabi_apply_orientation(window.rootViewController, window, 0);
    }
    nabi_mobile_window_mode_changed(state);
}

static void nabi_schedule_window_mode_refresh(void);
static char nabi_geometry_observation;

// This noninteractive view receives UIKit layout events without replacing any
// of winit's view/controller methods. It also works with the legacy nil scene.
@interface NabiWindowModeObserver : UIView
@property(nonatomic, weak) UIWindowScene *observedScene;
@end

@implementation NabiWindowModeObserver
- (void)updateSceneObservation {
    if (@available(iOS 16.0, *)) {
        UIWindowScene *scene = self.window.windowScene;
        if (scene == self.observedScene) return;
        [self.observedScene removeObserver:self forKeyPath:@"effectiveGeometry" context:&nabi_geometry_observation];
        self.observedScene = scene;
        [scene addObserver:self forKeyPath:@"effectiveGeometry" options:0 context:&nabi_geometry_observation];
    }
}
- (void)didMoveToWindow {
    [super didMoveToWindow];
    [self updateSceneObservation];
    nabi_schedule_window_mode_refresh();
}
- (void)layoutSubviews {
    [super layoutSubviews];
    [self updateSceneObservation];
    nabi_schedule_window_mode_refresh();
}
- (void)safeAreaInsetsDidChange {
    [super safeAreaInsetsDidChange];
    [self updateSceneObservation];
    nabi_schedule_window_mode_refresh();
}
- (void)windowEnvironmentDidChange:(NSNotification *)notification {
    (void)notification;
    nabi_schedule_window_mode_refresh();
}
- (void)observeValueForKeyPath:(NSString *)keyPath ofObject:(id)object
                       change:(NSDictionary<NSKeyValueChangeKey,id> *)change context:(void *)context {
    if (context == &nabi_geometry_observation) {
        nabi_schedule_window_mode_refresh();
    } else {
        [super observeValueForKeyPath:keyPath ofObject:object change:change context:context];
    }
}
- (void)dealloc {
    [NSNotificationCenter.defaultCenter removeObserver:self];
    [self.observedScene removeObserver:self forKeyPath:@"effectiveGeometry" context:&nabi_geometry_observation];
}
@end

static NabiWindowModeObserver *nabi_window_observer;
static void nabi_track_window(UIWindow *window) {
    if (!nabi_window_observer) {
        nabi_window_observer = [[NabiWindowModeObserver alloc] initWithFrame:CGRectZero];
        nabi_window_observer.opaque = NO;
        nabi_window_observer.userInteractionEnabled = NO;
        nabi_window_observer.isAccessibilityElement = NO;
        nabi_window_observer.accessibilityElementsHidden = YES;
        nabi_window_observer.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
        for (NSNotificationName name in @[
            UIApplicationDidBecomeActiveNotification, UIWindowDidBecomeKeyNotification,
            UIScreenDidConnectNotification, UIScreenDidDisconnectNotification, UIScreenModeDidChangeNotification
        ]) {
            [NSNotificationCenter.defaultCenter addObserver:nabi_window_observer
                selector:@selector(windowEnvironmentDidChange:) name:name object:nil];
        }
    }
    if (nabi_window_observer.window != window) {
        [nabi_window_observer removeFromSuperview];
        if (window) {
            nabi_window_observer.frame = window.bounds;
            [window insertSubview:nabi_window_observer atIndex:0];
        }
        nabi_publish_window_mode(window);
    } else if (nabi_window_mode == -2) {
        nabi_publish_window_mode(window);
    }
}

static void nabi_schedule_window_mode_refresh(void) {
    static BOOL pending;
    if (pending) return;
    pending = YES;
    dispatch_async(dispatch_get_main_queue(), ^{
        pending = NO;
        UIWindow *window = nabi_window();
        nabi_track_window(window);
        nabi_publish_window_mode(window);
    });
}

static UIInterfaceOrientationMask nabi_supported_orientations(id self, SEL command) {
    (void)self; (void)command; return nabi_orientations;
}
static BOOL nabi_home_indicator_hidden(id self, SEL command) {
    (void)self; (void)command; return nabi_immersive;
}
static void nabi_apply_orientation(UIViewController *controller, UIWindow *window,
                                   UIInterfaceOrientationMask restoreOrientation) {
    if (@available(iOS 16.0, *)) {
        [controller setNeedsUpdateOfSupportedInterfaceOrientations];
        if (window.windowScene) {
            // Allowing All (or asking for no preference) alone leaves the current
            // landscape orientation valid. On an explicit return to Auto, make a
            // one-shot request for the OS orientation preceding our override.
            // The supported mask stays All, so subsequent rotation belongs to iOS.
            UIInterfaceOrientationMask requested = restoreOrientation ?: nabi_orientations;
            UIWindowSceneGeometryPreferencesIOS *preferences =
                requested == UIInterfaceOrientationMaskAll
                ? [[UIWindowSceneGeometryPreferencesIOS alloc] init]
                : [[UIWindowSceneGeometryPreferencesIOS alloc] initWithInterfaceOrientations:requested];
            [window.windowScene requestGeometryUpdateWithPreferences:preferences errorHandler:^(NSError *error) {
                // A denied request does not turn a standalone window into a
                // windowed one. Keep this separate from icon visibility.
                NSLog(@"NABI VIEW: orientation request denied (%@:%ld)", error.domain, (long)error.code);
            }];
        }
    } else {
        [UIViewController attemptRotationToDeviceOrientation];
    }
}
static UIViewController *nabi_controller(UIWindow *window, BOOL applyInitialOrientation) {
    static Class installed;
    static __weak UIViewController *initializedController;
    nabi_track_window(window);
    UIViewController *controller = window.rootViewController;
    if (!controller) return nil;
    Class cls = object_getClass(controller);
    if (cls != installed) {
        class_replaceMethod(cls, @selector(supportedInterfaceOrientations), (IMP)nabi_supported_orientations, "Q@:");
        class_replaceMethod(cls, @selector(prefersHomeIndicatorAutoHidden), (IMP)nabi_home_indicator_hidden, "B@:");
        installed = cls;
    }
    if (controller != initializedController) {
        initializedController = controller;
        // A lock requested before the window existed applies now.
        if (applyInitialOrientation) nabi_apply_orientation(controller, window, 0);
    }
    return controller;
}

void nabi_mobile_set_orientation(int mode) {
    dispatch_async(dispatch_get_main_queue(), ^{
        UIWindow *window = nabi_window();
        nabi_track_window(window);
        // The queued request may have preceded a resize or a scene move.
        int windowMode = nabi_detect_window_mode(window);
        int effectiveMode = windowMode == 1 ? 0 : mode;
        UIWindowScene *scene = window.windowScene;
        if (scene != nabi_orientation_scene) {
            nabi_orientation_scene = scene;
            nabi_system_orientation = UIInterfaceOrientationUnknown;
        }
        BOOL wasAuto = nabi_orientations == UIInterfaceOrientationMaskAll;
        if (wasAuto && effectiveMode != 0 && windowMode == 0) {
            if (@available(iOS 16.0, *)) {
                nabi_system_orientation = scene.effectiveGeometry.interfaceOrientation;
            } else {
                nabi_system_orientation = scene.interfaceOrientation;
            }
        }
        UIInterfaceOrientationMask restoreOrientation = 0;
        if (!wasAuto && effectiveMode == 0 && windowMode == 0 &&
            (UIInterfaceOrientationIsPortrait(nabi_system_orientation) ||
             UIInterfaceOrientationIsLandscape(nabi_system_orientation))) {
            restoreOrientation = (UIInterfaceOrientationMask)1 << nabi_system_orientation;
        }
        nabi_orientations = effectiveMode == 1 ? (UIInterfaceOrientationMaskPortrait | UIInterfaceOrientationMaskPortraitUpsideDown)
            : effectiveMode == 2 ? UIInterfaceOrientationMaskLandscape : UIInterfaceOrientationMaskAll;
        UIViewController *controller = nabi_controller(window, NO);
        if (controller) nabi_apply_orientation(controller, window, restoreOrientation);
        nabi_publish_window_mode(window);
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
        UIViewController *controller = nabi_controller(nabi_window(), YES);
        [controller setNeedsUpdateOfHomeIndicatorAutoHidden];
    });
}

static UIViewController *nabi_presenter(void) {
    UIViewController *presenter = nabi_window().rootViewController;
    while (presenter.presentedViewController) presenter = presenter.presentedViewController;
    return presenter;
}

static NSString *nabi_text(const char *key) {
    size_t length = nabi_mobile_text(key, NULL, 0);
    if (!length || length > 16384) return [NSString stringWithUTF8String:key];
    NSMutableData *data = [NSMutableData dataWithLength:length * sizeof(unichar)];
    size_t written = nabi_mobile_text(key, data.mutableBytes, length);
    return written == length ? [[NSString alloc] initWithCharacters:data.bytes length:length]
        : [NSString stringWithUTF8String:key];
}

static void nabi_share(const unsigned short *text, int length, BOOL disposable) {
    if (length < 0) return;
    NSString *path = [[NSString alloc] initWithCharacters:text length:(NSUInteger)length];
    dispatch_async(dispatch_get_main_queue(), ^{
        UIViewController *presenter = nabi_presenter();
        if (!presenter) {
            if (disposable) [NSFileManager.defaultManager removeItemAtPath:path error:nil];
            return;
        }
        UIActivityViewController *sheet = [[UIActivityViewController alloc]
            initWithActivityItems:@[[NSURL fileURLWithPath:path]] applicationActivities:nil];
        // iPad presents the sheet as a popover anchored above the bottom bar.
        CGRect bounds = presenter.view.bounds;
        sheet.popoverPresentationController.sourceView = presenter.view;
        sheet.popoverPresentationController.sourceRect = CGRectMake(CGRectGetMidX(bounds), CGRectGetMaxY(bounds) - 48., 1., 1.);
        if (disposable) sheet.completionWithItemsHandler = ^(UIActivityType type, BOOL completed, NSArray *items, NSError *error) {
            (void)type; (void)completed; (void)items; (void)error;
            [NSFileManager.defaultManager removeItemAtPath:path error:nil];
        };
        [presenter presentViewController:sheet animated:YES completion:nil];
    });
}

void nabi_mobile_share(const unsigned short *text, int length) { nabi_share(text, length, NO); }
void nabi_mobile_share_owned(const unsigned short *text, int length) { nabi_share(text, length, YES); }

void nabi_mobile_safe_area(float *out) {
    UIWindow *window = nabi_window();
    nabi_controller(window, YES);
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
@property(nonatomic) BOOL discoverKind;
@property(nonatomic, copy) NSString *reconnecting;
@end

static BOOL nabi_valid_source_id(NSString *identifier) {
    if (identifier.length < 8 || identifier.length > 64) return NO;
    NSCharacterSet *allowed = [NSCharacterSet characterSetWithCharactersInString:@"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-"];
    return [identifier rangeOfCharacterFromSet:allowed.invertedSet].location == NSNotFound;
}

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
static NSString *nabi_save_source(NSArray<NSURL *> *urls, BOOL folder, NSString *reconnecting) {
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
    NSString *identifier = reconnecting;
    if (!identifier) {
        for (NSString *key in grants) {
            NSDictionary *record = grants[key];
            if ([record isKindOfClass:NSDictionary.class] && [record[@"folder"] boolValue] == folder && [record[@"locations"] isEqual:locations]) {
                identifier = key; break;
            }
        }
    }
    if (!identifier) identifier = NSUUID.UUID.UUIDString;
    grants[identifier] = @{@"bookmarks":bookmarks, @"locations":locations, @"folder":@(folder), @"name":urls.firstObject.lastPathComponent ?: @"Selected documents"};
    return nabi_write_grants(grants) ? identifier : nil;
}

// Keep permission/missing-source recovery separate from transient provider I/O.
static int nabi_document_error(NSError *error, int fileError) {
    if (fileError == EACCES || fileError == EPERM || fileError == ENOENT) return 6;
    if ([error.domain isEqualToString:NSCocoaErrorDomain] &&
        (error.code == NSFileReadNoPermissionError || error.code == NSFileNoSuchFileError ||
         error.code == NSFileReadNoSuchFileError)) return 6;
    return 7;
}

static void nabi_report_source(NSString *identifier, NSString *fallback) {
    if (!identifier) return;
    NSDictionary *record = nabi_grants()[identifier];
    if (![record isKindOfClass:NSDictionary.class]) return;
    NSString *name = [record[@"name"] isKindOfClass:NSString.class] ? record[@"name"] : fallback;
    nabi_mobile_import_source(identifier.UTF8String, name.UTF8String);
}

// Called only on Rust workers. Native grants remain in the host and are
// resolved for each requested file; no original URL reaches source markers.
int nabi_mobile_read_source(const char *source, const char *item, int output, uint64_t limit,
                            bool (*live)(void *), void *context) {
    @autoreleasepool {
        if (!source || !item || !live || !live(context)) return -1;
        NSString *identifier = [NSString stringWithUTF8String:source];
        NSString *token = [NSString stringWithUTF8String:item];
        if (!nabi_valid_source_id(identifier) || !token.length) return 6;
        if (nabi_photos_is_collection(identifier)) return nabi_photos_read(token, output, limit, live, context);
        // Tokens are one enumerated child name, never an arbitrary path.
        if (![token.lastPathComponent isEqualToString:token] || [@[@".", @".."] containsObject:token]) return 6;
        NSDictionary *record = nabi_grants()[identifier];
        NSArray *bookmarks = [record isKindOfClass:NSDictionary.class] ? record[@"bookmarks"] : nil;
        if (![record[@"folder"] boolValue] || ![bookmarks isKindOfClass:NSArray.class] || bookmarks.count != 1 ||
            ![bookmarks.firstObject isKindOfClass:NSData.class]) return 6;
        BOOL stale = NO;
        NSURL *folder = [NSURL URLByResolvingBookmarkData:bookmarks.firstObject options:NSURLBookmarkResolutionWithoutUI
            relativeToURL:nil bookmarkDataIsStale:&stale error:nil];
        if (!folder) return 6;
        BOOL access = [folder startAccessingSecurityScopedResource];
        NSURL *url = [folder URLByAppendingPathComponent:token isDirectory:NO];
        NSFileCoordinator *coordinator = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
        NSError *error = nil;
        __block int result = 7;
        [coordinator coordinateReadingItemAtURL:url options:0 error:&error byAccessor:^(NSURL *readURL) {
            if (!live(context)) { result = -1; return; }
            int fd = open(readURL.fileSystemRepresentation, O_RDONLY | O_CLOEXEC);
            if (fd < 0) { result = nabi_document_error(nil, errno); return; }
            struct stat info;
            if (fstat(fd, &info) < 0 || !S_ISREG(info.st_mode) || info.st_size < 0 || (uint64_t)info.st_size > limit) {
                close(fd); return;
            }
            uint8_t buffer[64 * 1024];
            uint64_t bytes = 0;
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:15];
            while (live(context) && deadline.timeIntervalSinceNow > 0) {
                ssize_t count = read(fd, buffer, sizeof(buffer));
                if (count < 0 && errno == EINTR) continue;
                if (count < 0 || (uint64_t)count > limit - bytes) break;
                if (count == 0) { result = bytes ? 0 : 7; break; }
                size_t offset = 0;
                while (offset < (size_t)count) {
                    ssize_t written = write(output, buffer + offset, (size_t)count - offset);
                    if (written < 0 && errno == EINTR) continue;
                    if (written <= 0) break;
                    offset += (size_t)written;
                }
                if (offset != (size_t)count) break;
                bytes += (uint64_t)count;
            }
            if (!live(context)) result = -1;
            if (result == 0) {
                struct timespec times[2] = {info.st_atimespec, info.st_mtimespec};
                futimens(output, times);
            }
            close(fd);
        }];
        if (access) [folder stopAccessingSecurityScopedResource];
        return error ? nabi_document_error(error, 0) : result;
    }
}

static void nabi_import_folder(NSURL *url, NSString *identifier) {
    BOOL access = [url startAccessingSecurityScopedResource];
    __block NSError *listError = nil;
    NSError *coordinationError = nil;
    __block NSArray<NSURL *> *items = nil;
    NSFileCoordinator *listing = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
    [listing coordinateReadingItemAtURL:url options:0 error:&coordinationError byAccessor:^(NSURL *readURL) {
        items = [NSFileManager.defaultManager contentsOfDirectoryAtURL:readURL
            includingPropertiesForKeys:@[NSURLIsRegularFileKey, NSURLFileSizeKey, NSURLContentModificationDateKey]
            options:NSDirectoryEnumerationSkipsHiddenFiles error:&listError];
    }];
    if (!nabi_mobile_import_begin_folder()) items = @[];
    nabi_report_source(identifier, url.lastPathComponent);
    int failure = 0;
    NSUInteger readable = 0;
    if (listError || coordinationError) nabi_mobile_photo_error(nabi_document_error(listError ?: coordinationError, 0));
    for (NSURL *item in items) {
        NSNumber *regular = nil;
        [item getResourceValue:&regular forKey:NSURLIsRegularFileKey error:nil];
        if (!regular.boolValue) continue;
        if (identifier) {
            NSNumber *size = nil;
            NSDate *modified = nil;
            [item getResourceValue:&size forKey:NSURLFileSizeKey error:nil];
            [item getResourceValue:&modified forKey:NSURLContentModificationDateKey error:nil];
            readable++;
            if (!nabi_mobile_import_deferred(item.lastPathComponent.UTF8String, item.lastPathComponent.UTF8String,
                                            size.unsignedLongLongValue, modified.timeIntervalSince1970, 0, 0)) break;
            continue;
        }
        // Rust consumes and closes the descriptor and skips unsupported files.
        __block BOOL more = YES;
        __block int fileError = 0;
        __block BOOL opened = NO;
        NSError *readError = nil;
        NSFileCoordinator *coordinator = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
        [coordinator coordinateReadingItemAtURL:item options:0 error:&readError byAccessor:^(NSURL *readURL) {
            int fd = open(readURL.fileSystemRepresentation, O_RDONLY | O_CLOEXEC);
            if (fd >= 0) { opened = YES; more = nabi_mobile_import_file(fd, item.lastPathComponent.UTF8String); }
            else fileError = errno;
        }];
        if (opened) readable++;
        else if (readError || fileError) failure = nabi_document_error(readError, fileError);
        if (!more) break;
    }
    if (!readable && failure) nabi_mobile_photo_error(failure);
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
        __block int fileError = 0;
        [coordinator coordinateReadingItemAtURL:url options:0 error:&error byAccessor:^(NSURL *readURL) {
            int fd = open(readURL.fileSystemRepresentation, O_RDONLY | O_CLOEXEC);
            if (fd >= 0) copied = nabi_mobile_import_file(fd, url.lastPathComponent.UTF8String);
            else fileError = errno;
        }];
        if (access) [url stopAccessingSecurityScopedResource];
        if (error || fileError) nabi_mobile_photo_error(nabi_document_error(error, fileError));
        ok = copied && error == nil;
    }
    nabi_mobile_import_finish();
}

@implementation NabiDocumentPicker
- (void)documentPickerWasCancelled:(UIDocumentPickerViewController *)controller {
    (void)controller;
    BOOL reconnecting = self.reconnecting != nil;
    self.reconnecting = nil; self.busy = NO;
    if (reconnecting) nabi_mobile_access_action_result(3, false);
}
- (void)documentPicker:(UIDocumentPickerViewController *)controller didPickDocumentsAtURLs:(NSArray<NSURL *> *)urls {
    BOOL folder = self.folder;
    NSString *reconnecting = self.reconnecting;
    if (self.discoverKind && urls.count) {
        NSURL *url = urls.firstObject;
        BOOL access = [url startAccessingSecurityScopedResource];
        NSNumber *directory = nil;
        NSError *error = nil;
        BOOL known = [url getResourceValue:&directory forKey:NSURLIsDirectoryKey error:&error];
        if (access) [url stopAccessingSecurityScopedResource];
        if (!known || ![directory isKindOfClass:NSNumber.class]) {
            nabi_mobile_import_begin();
            nabi_mobile_photo_error(nabi_document_error(error, 0));
            nabi_mobile_import_finish();
            self.reconnecting = nil; self.busy = NO;
            if (reconnecting) nabi_mobile_access_action_result(3, true);
            return;
        }
        folder = directory.boolValue;
    }
    NSArray<NSURL *> *selection = folder && urls.count ? @[urls.firstObject] : urls;
    if (!selection.count) {
        [self documentPickerWasCancelled:controller];
        return;
    }
    void (^import)(void) = ^{
        nabi_mobile_import_pending();
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
            NSString *identifier = nabi_save_source(selection, folder, reconnecting);
            if (reconnecting && !identifier) {
                nabi_mobile_import_begin(); nabi_mobile_photo_error(6); nabi_mobile_import_finish();
            } else if (folder) nabi_import_folder(selection.firstObject, identifier);
            else nabi_import_documents(selection, identifier);
            dispatch_async(dispatch_get_main_queue(), ^{
                self.reconnecting = nil; self.busy = NO;
                if (reconnecting) nabi_mobile_access_action_result(3, true);
            });
        });
    };
    NSDictionary *previous = reconnecting ? nabi_grants()[reconnecting] : nil;
    BOOL same = [previous isKindOfClass:NSDictionary.class] &&
        [previous[@"locations"] isEqual:[selection valueForKey:@"absoluteString"]];
    if (!reconnecting || same) { import(); return; }
    // Do not replace an existing source with an unrelated folder silently.
    // Missing previous locations also require an explicit reconnection choice.
    [controller dismissViewControllerAnimated:YES completion:^{
        UIViewController *presenter = nabi_presenter();
        if (!presenter) { [self documentPickerWasCancelled:controller]; return; }
        UIAlertController *confirmation = [UIAlertController alertControllerWithTitle:nabi_text("access-reconnect-title")
            message:nabi_text("access-reconnect-message") preferredStyle:UIAlertControllerStyleAlert];
        [confirmation addAction:[UIAlertAction actionWithTitle:nabi_text("cancel") style:UIAlertActionStyleCancel handler:^(UIAlertAction *action) {
            (void)action; [self documentPickerWasCancelled:controller];
        }]];
        [confirmation addAction:[UIAlertAction actionWithTitle:nabi_text("access-reconnect") style:UIAlertActionStyleDefault handler:^(UIAlertAction *action) {
            (void)action; import();
        }]];
        [presenter presentViewController:confirmation animated:YES completion:nil];
    }];
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
    if (!nabi_valid_source_id(identifier)) return false;
    __block BOOL accepted = NO;
    void (^start)(void) = ^{
        NabiDocumentPicker *delegate = nabi_document_picker();
        if (delegate.busy) return;
        delegate.busy = YES; accepted = YES;
        if (nabi_photos_is_collection(identifier)) {
            nabi_photos_open(identifier, ^{
                dispatch_async(dispatch_get_main_queue(), ^{ delegate.busy = NO; });
            });
            return;
        }
        nabi_mobile_import_pending();
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
                    nabi_mobile_photo_error(6);
                    nabi_mobile_import_finish();
                }
                dispatch_async(dispatch_get_main_queue(), ^{ delegate.busy = NO; });
            }
        });
    };
    if (NSThread.isMainThread) start(); else dispatch_sync(dispatch_get_main_queue(), start);
    return accepted;
}

static BOOL nabi_pick_documents(BOOL folder, NSString *reconnecting, BOOL discoverKind) {
    __block BOOL accepted = NO;
    void (^start)(void) = ^{
        NabiDocumentPicker *delegate = nabi_document_picker();
        if (delegate.busy) return;
        UIViewController *presenter = nabi_presenter();
        if (!presenter) return;
        // Generic data also exposes AVIF/JXL/RAW extensions not registered with
        // the OS. The shared Rust import layer validates supported extensions.
        NSArray<UTType *> *types = discoverKind ? @[UTTypeFolder, UTTypeImage, UTTypeData]
            : folder ? @[UTTypeFolder] : @[UTTypeImage, UTTypeData];
        UIDocumentPickerViewController *picker = [[UIDocumentPickerViewController alloc]
            initForOpeningContentTypes:types asCopy:NO];
        picker.allowsMultipleSelection = !folder && !discoverKind;
        picker.delegate = delegate;
        delegate.folder = folder;
        delegate.discoverKind = discoverKind;
        delegate.reconnecting = reconnecting;
        delegate.busy = YES;
        accepted = YES;
        [presenter presentViewController:picker animated:YES completion:nil];
    };
    if (NSThread.isMainThread) start(); else dispatch_sync(dispatch_get_main_queue(), start);
    return accepted;
}

void nabi_mobile_pick(void) { nabi_pick_documents(NO, nil, NO); }
void nabi_mobile_pick_folder(void) { nabi_pick_documents(YES, nil, NO); }

bool nabi_mobile_reselect(const unsigned short *text, int length) {
    if (length < 8 || length > 64) return false;
    NSString *identifier = [[NSString alloc] initWithCharacters:text length:(NSUInteger)length];
    if (!nabi_valid_source_id(identifier) || nabi_photos_is_collection(identifier)) return false;
    NSDictionary *record = nabi_grants()[identifier];
    if (![record isKindOfClass:NSDictionary.class] || ![record[@"folder"] isKindOfClass:NSNumber.class]) {
        // A missing native grant may still have its local source description.
        NSString *path = [[NSString stringWithUTF8String:nabi_mobile_path(0)]
            stringByAppendingPathComponent:[NSString stringWithFormat:@"document-sources/%@/.nabi-document-source.json", identifier]];
        NSData *data = [NSData dataWithContentsOfFile:path options:NSDataReadingMappedIfSafe error:nil];
        id marker = data.length <= 1024 * 1024 ? [NSJSONSerialization JSONObjectWithData:data ?: [NSData data] options:0 error:nil] : nil;
        record = [marker isKindOfClass:NSDictionary.class] && [marker[@"id"] isEqual:identifier] ? marker : nil;
    }
    // A library entry may outlive both its marker and its native bookmark.
    // Ask for one file or folder, then determine its kind from the selected
    // URL. The explicit reconnect confirmation still protects the old entry.
    BOOL discoverKind = ![record[@"folder"] isKindOfClass:NSNumber.class];
    return nabi_pick_documents([record[@"folder"] boolValue], identifier, discoverKind);
}

bool nabi_mobile_open_settings(void) {
    __block BOOL accepted = NO;
    void (^start)(void) = ^{
        NSURL *url = [NSURL URLWithString:UIApplicationOpenSettingsURLString];
        NabiDocumentPicker *delegate = nabi_document_picker();
        if (!url || delegate.busy) return;
        delegate.busy = YES; accepted = YES;
        [UIApplication.sharedApplication openURL:url options:@{} completionHandler:^(BOOL success) {
            dispatch_async(dispatch_get_main_queue(), ^{
                delegate.busy = NO;
                nabi_mobile_access_action_result(1, success);
            });
        }];
    };
    if (NSThread.isMainThread) start(); else dispatch_sync(dispatch_get_main_queue(), start);
    return accepted;
}

bool nabi_mobile_manage_photos(void) {
    __block BOOL accepted = NO;
    void (^start)(void) = ^{
        NabiDocumentPicker *delegate = nabi_document_picker();
        UIViewController *presenter = nabi_presenter();
        if (delegate.busy || !presenter || nabi_mobile_photo_status() != 4) return;
        delegate.busy = YES; accepted = YES;
        [PHPhotoLibrary.sharedPhotoLibrary presentLimitedLibraryPickerFromViewController:presenter completionHandler:^(NSArray<NSString *> *identifiers) {
            (void)identifiers;
            dispatch_async(dispatch_get_main_queue(), ^{
                delegate.busy = NO;
                nabi_mobile_access_action_result(2, true);
                nabi_mobile_access_changed();
            });
        }];
    };
    if (NSThread.isMainThread) start(); else dispatch_sync(dispatch_get_main_queue(), start);
    return accepted;
}

bool nabi_mobile_request_photo_access(void) {
    __block BOOL accepted = NO;
    void (^start)(void) = ^{
        NabiDocumentPicker *delegate = nabi_document_picker();
        if (delegate.busy || nabi_mobile_photo_status() != 0) return;
        delegate.busy = YES; accepted = YES;
        [PHPhotoLibrary requestAuthorizationForAccessLevel:PHAccessLevelReadWrite handler:^(PHAuthorizationStatus status) {
            (void)status; // The Rust UI obtains the current status after completion.
            dispatch_async(dispatch_get_main_queue(), ^{
                delegate.busy = NO;
                nabi_mobile_access_action_result(4, true);
                nabi_mobile_access_changed();
            });
        }];
    };
    if (NSThread.isMainThread) start(); else dispatch_sync(dispatch_get_main_queue(), start);
    return accepted;
}

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
// Scaled while decoding (ImageIO thumbnail path, EXIF transform applied) and
// drawn once into a caller-allocated sRGB buffer as straight-alpha RGBA.
// original receives the upright header size. 0 is success.
typedef unsigned char *(*nabi_scaled_alloc)(void *, size_t);
int nabi_decode_image_scaled(const char *path, unsigned int max_side, unsigned long long max_pixels,
        nabi_scaled_alloc alloc, void *context, unsigned int *width, unsigned int *height,
        unsigned int *original) {
    @autoreleasepool {
        if (!max_side || !max_pixels) return -1;
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        NSDictionary *sourceOptions = @{(__bridge NSString *)kCGImageSourceShouldCache: @NO};
        CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url,
            (__bridge CFDictionaryRef)sourceOptions);
        if (!source) return -1;
        NSDictionary *properties = CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source, 0, NULL));
        uint64_t w = [properties[(__bridge NSString *)kCGImagePropertyPixelWidth] unsignedLongLongValue];
        uint64_t h = [properties[(__bridge NSString *)kCGImagePropertyPixelHeight] unsignedLongLongValue];
        NSInteger orientation = [properties[(__bridge NSString *)kCGImagePropertyOrientation] integerValue];
        if (!w || !h || w * h > 256000000ULL) { CFRelease(source); return -2; }
        double scale = MIN(1.0, MIN((double)max_side / (double)MAX(w, h),
            sqrt((double)max_pixels / ((double)w * (double)h))));
        NSUInteger edge = MAX((NSUInteger)1, (NSUInteger)floor((double)MAX(w, h) * scale));
        NSDictionary *options = @{
            (__bridge NSString *)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
            (__bridge NSString *)kCGImageSourceCreateThumbnailWithTransform: @YES,
            (__bridge NSString *)kCGImageSourceShouldCacheImmediately: @YES,
            (__bridge NSString *)kCGImageSourceThumbnailMaxPixelSize: @(edge),
        };
        CGImageRef image = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
        CFRelease(source);
        if (!image) return -3;
        size_t ow = CGImageGetWidth(image), oh = CGImageGetHeight(image);
        unsigned char *data = ow && oh && ow <= max_side && oh <= max_side ? alloc(context, ow * oh * 4) : NULL;
        if (!data) { CGImageRelease(image); return -4; }
        CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef bitmap = CGBitmapContextCreate(data, ow, oh, 8, ow * 4, color,
            kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
        CGColorSpaceRelease(color);
        if (!bitmap) { CGImageRelease(image); return -4; }
        CGContextSetBlendMode(bitmap, kCGBlendModeCopy);
        CGContextDrawImage(bitmap, CGRectMake(0, 0, ow, oh), image);
        CGContextRelease(bitmap); CGImageRelease(image);
        for (size_t i = 0; i < ow * oh; ++i) {
            unsigned a = data[i * 4 + 3];
            if (a && a < 255) for (int c = 0; c < 3; ++c) data[i * 4 + c] = MIN(255, (data[i * 4 + c] * 255 + a / 2) / a);
        }
        // EXIF orientations 5-8 swap the axes.
        BOOL swapped = orientation >= 5 && orientation <= 8;
        *width = (unsigned int)ow; *height = (unsigned int)oh;
        original[0] = (unsigned int)(swapped ? h : w); original[1] = (unsigned int)(swapped ? w : h);
        return 0;
    }
}

void nabi_mobile_copy_text(const unsigned short *text, int length) {
    if (length < 0) return;
    NSString *value = [[NSString alloc] initWithCharacters:text length:(NSUInteger)length];
    dispatch_async(dispatch_get_main_queue(), ^{ UIPasteboard.generalPasteboard.string = value; });
}
