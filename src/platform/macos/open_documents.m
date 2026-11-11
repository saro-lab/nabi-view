#import <AppKit/AppKit.h>
#import <Carbon/Carbon.h>

extern void nabi_enqueue_open_document(const char *path);
extern void nabi_reopen_application(void);
extern void nabi_request_close_all(void);
extern void nabi_request_about(void);
extern void nabi_finished_launching(void);
extern void nabi_stop_open_events(void);
extern void nabi_access_capture_url(NSURL *url);

@interface NabiOpenDocuments : NSObject
- (void)configureMenu;
@end
static NabiOpenDocuments *handler;
static BOOL shortcutCapture;
static NSMapTable<NSMenuItem *, NSString *> *savedKeys;

static void captureMenu(NSMenu *menu, BOOL capture) {
    for (NSMenuItem *item in menu.itemArray) {
        if (capture && item.keyEquivalent.length) {
            if (![savedKeys objectForKey:item]) [savedKeys setObject:item.keyEquivalent forKey:item];
            item.keyEquivalent = @"";
        }
        if (item.submenu) captureMenu(item.submenu, capture);
    }
}

void nabi_set_shortcut_capture(bool active) {
    shortcutCapture = active;
    if (!savedKeys) savedKeys = [NSMapTable weakToStrongObjectsMapTable];
    if (active) {
        captureMenu(NSApp.mainMenu, YES);
    } else {
        for (NSMenuItem *item in savedKeys.keyEnumerator) item.keyEquivalent = [savedKeys objectForKey:item];
        [savedKeys removeAllObjects];
    }
}

void nabi_configure_menu(void) { [handler configureMenu]; }

// Titles follow the viewer preference; actions and user-remapped shortcuts stay intact.
void nabi_localize_menu(const char *about, const char *hide, const char *others,
                        const char *show, const char *quit, const char *services) {
    @autoreleasepool {
        for (NSMenuItem *top in NSApp.mainMenu.itemArray) {
            for (NSMenuItem *item in top.submenu.itemArray) {
                const char *title = NULL;
                if (item.action == @selector(showAbout:) || item.action == @selector(orderFrontStandardAboutPanel:)) title = about;
                else if (item.action == @selector(hide:)) title = hide;
                else if (item.action == @selector(hideOtherApplications:)) title = others;
                else if (item.action == @selector(unhideAllApplications:)) title = show;
                else if (item.action == @selector(closeAll:) || item.action == @selector(terminate:)) title = quit;
                else if (item.submenu && item.submenu == NSApp.servicesMenu) title = services;
                if (title) item.title = [NSString stringWithUTF8String:title];
            }
        }
    }
}

@implementation NabiOpenDocuments
- (void)registerHandler:(NSNotification *)notification {
    [[NSAppleEventManager sharedAppleEventManager]
        setEventHandler:self andSelector:@selector(openDocuments:reply:)
        forEventClass:kCoreEventClass andEventID:kAEOpenDocuments];
    [[NSAppleEventManager sharedAppleEventManager]
        setEventHandler:self andSelector:@selector(reopenApplication:reply:)
        forEventClass:kCoreEventClass andEventID:kAEReopenApplication];
}

- (void)reopenApplication:(NSAppleEventDescriptor *)event reply:(NSAppleEventDescriptor *)reply {
    nabi_reopen_application();
}

- (void)didFinishLaunching:(NSNotification *)notification {
    nabi_finished_launching();
    [self configureMenu];
}

- (void)configureMenu {
    for (NSMenuItem *top in NSApp.mainMenu.itemArray) {
        for (NSMenuItem *item in top.submenu.itemArray) {
            if (item.action == @selector(orderFrontStandardAboutPanel:)) {
                item.target = self;
                item.action = @selector(showAbout:);
            } else if (item.action == @selector(terminate:)) {
                item.target = self;
                item.action = @selector(closeAll:);
                // The configurable Rust shortcut owns Command-Q.
                item.keyEquivalent = @"";
            }
        }
    }
}

- (void)closeAll:(id)sender { if (!shortcutCapture) nabi_request_close_all(); }
- (void)showAbout:(id)sender { nabi_request_about(); }

- (void)willTerminate:(NSNotification *)notification {
    nabi_stop_open_events();
}

- (void)openDocuments:(NSAppleEventDescriptor *)event reply:(NSAppleEventDescriptor *)reply {
    NSAppleEventDescriptor *files = [event paramDescriptorForKeyword:keyDirectObject];
    for (NSInteger index = 1; index <= files.numberOfItems; index++) {
        NSAppleEventDescriptor *entry = [files descriptorAtIndex:index];
        NSURL *url = [entry coerceToDescriptorType:typeFileURL].fileURLValue;
        if (url.isFileURL && url.fileSystemRepresentation != NULL) {
            nabi_access_capture_url(url);
            nabi_enqueue_open_document(url.fileSystemRepresentation);
        }
    }
}
@end

bool nabi_launch_window(const char *bundle, const char *path) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:bundle isDirectory:YES relativeToURL:nil];
        NSMutableArray *args = [NSMutableArray arrayWithObject:@"--new-window"];
        if (path) {
            NSString *argument = [[NSFileManager defaultManager] stringWithFileSystemRepresentation:path length:strlen(path)];
            if (!argument) return false;
            [args addObject:argument];
        }
        // Keep the synchronous error contract without blocking a completion
        // handler on the main thread. Internal windows are not recent apps.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        NSRunningApplication *app = [[NSWorkspace sharedWorkspace]
            launchApplicationAtURL:url
            options:NSWorkspaceLaunchNewInstance | NSWorkspaceLaunchWithoutAddingToRecents
            configuration:@{NSWorkspaceLaunchConfigurationArguments: args}
            error:nil];
#pragma clang diagnostic pop
        return app != nil;
    }
}

void nabi_install_open_documents(void) {
    // Register during AppKit startup, before the initial open-documents event.
    // Leave the event-loop library's application delegate in place.
    if (handler != nil) return;
    handler = [NabiOpenDocuments new];
    [[NSNotificationCenter defaultCenter]
        addObserver:handler selector:@selector(registerHandler:)
        name:NSApplicationWillFinishLaunchingNotification object:nil];
    [[NSNotificationCenter defaultCenter]
        addObserver:handler selector:@selector(didFinishLaunching:)
        name:NSApplicationDidFinishLaunchingNotification object:nil];
    [[NSNotificationCenter defaultCenter]
        addObserver:handler selector:@selector(willTerminate:)
        name:NSApplicationWillTerminateNotification object:nil];
}
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
int nabi_default_for_extension(const char *extension) {
    @autoreleasepool {
        UTType *type = [UTType typeWithFilenameExtension:[NSString stringWithUTF8String:extension]];
        if (!type) return -1;
        NSURL *url = [[NSWorkspace sharedWorkspace] URLForApplicationToOpenContentType:type];
        if (!url) return 0;
        return [url.path.stringByStandardizingPath isEqualToString:NSBundle.mainBundle.bundleURL.path.stringByStandardizingPath] ? 1 : 0;
    }
}
int nabi_connect_extension(const char *extension) {
    @autoreleasepool {
        UTType *type = [UTType typeWithFilenameExtension:[NSString stringWithUTF8String:extension]];
        if (!type) return -1;
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        __block int result = -1;
        [[NSWorkspace sharedWorkspace] setDefaultApplicationAtURL:NSBundle.mainBundle.bundleURL toOpenContentType:type completionHandler:^(NSError *error) { result = error ? -1 : 0; dispatch_semaphore_signal(done); }];
        if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 30 * NSEC_PER_SEC))) return -1;
        return result;
    }
}
#import <ImageIO/ImageIO.h>
int nabi_decode_image(const char *path, unsigned char **pixels, unsigned int *width, unsigned int *height) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithPath:[NSString stringWithUTF8String:path]];
        CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, NULL);
        if (!source) return -1;
        NSDictionary *props = CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source, 0, NULL));
        NSUInteger w = [props[(__bridge NSString *)kCGImagePropertyPixelWidth] unsignedIntegerValue];
        NSUInteger h = [props[(__bridge NSString *)kCGImagePropertyPixelHeight] unsignedIntegerValue];
        if (!w || !h || w > 32768 || h > 32768 || w*h > 64000000) { CFRelease(source); return -1; }
        CGImageRef image = CGImageSourceCreateImageAtIndex(source, 0, NULL);
        CFRelease(source);
        if (!image) return -1;
        w = CGImageGetWidth(image); h = CGImageGetHeight(image);
        if (!w || !h || w > 32768 || h > 32768 || w*h > 64000000) { CGImageRelease(image); return -1; }
        unsigned char *data = calloc(w*h, 4);
        CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef context = CGBitmapContextCreate(data,w,h,8,w*4,color,kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
        CGColorSpaceRelease(color);
        if (!context) { free(data); CGImageRelease(image); return -1; }
        CGContextDrawImage(context,CGRectMake(0,0,w,h),image);
        CGContextRelease(context); CGImageRelease(image);
        for (NSUInteger i=0;i<w*h;i++) { unsigned int a=data[i*4+3]; if(a) for(int c=0;c<3;c++) data[i*4+c]=(unsigned char)MIN(255,(data[i*4+c]*255+a/2)/a); }
        *pixels=data; *width=(unsigned int)w; *height=(unsigned int)h; return 0;
    }
}
void nabi_free_image(unsigned char *pixels) { free(pixels); }
