#import <AppKit/AppKit.h>

// One viewer window per process. Stay on the current desktop instead of creating
// a native full-screen Space, which can blank other screens via system settings.
@interface NabiFullscreen : NSObject
@property(nonatomic, weak) NSWindow *window;
@property(nonatomic) NSRect savedFrame;
@property(nonatomic) BOOL savedMovable;
@property(nonatomic) NSApplicationPresentationOptions savedPresentation;
@property(nonatomic) BOOL blackout;
@property(nonatomic, strong) NSMutableArray<NSWindow *> *shades;
@end

@implementation NabiFullscreen
- (instancetype)init {
    if ((self = [super init])) {
        _shades = [NSMutableArray array];
        NSNotificationCenter *center = NSNotificationCenter.defaultCenter;
        for (NSString *name in @[NSApplicationDidChangeScreenParametersNotification,
                                 NSApplicationDidBecomeActiveNotification,
                                 NSApplicationDidResignActiveNotification,
                                 NSWindowDidMiniaturizeNotification, NSWindowDidDeminiaturizeNotification]) {
            [center addObserver:self selector:@selector(refresh:) name:name object:nil];
        }
        [center addObserver:self selector:@selector(closing:) name:NSWindowWillCloseNotification object:nil];
    }
    return self;
}
- (void)clearShades {
    for (NSWindow *shade in self.shades) [shade orderOut:nil];
    [self.shades removeAllObjects];
}
- (void)refresh:(NSNotification *)notification {
    NSWindow *window = self.window;
    if (!window) return;
    [self clearShades];
    BOOL presenting = NSApp.active && !window.miniaturized;
    NSApp.presentationOptions = presenting
        ? (NSApplicationPresentationAutoHideDock | NSApplicationPresentationAutoHideMenuBar)
        : self.savedPresentation;
    NSScreen *screen = window.screen ?: NSScreen.mainScreen;
    if (!screen) return;
    [window setFrame:screen.frame display:YES];
    if (!self.blackout || !presenting) return;
    for (NSScreen *other in NSScreen.screens) {
        if (other == screen || NSEqualRects(other.frame, screen.frame)) continue;
        NSWindow *shade = [[NSWindow alloc] initWithContentRect:other.frame
            styleMask:NSWindowStyleMaskBorderless backing:NSBackingStoreBuffered defer:NO];
        shade.releasedWhenClosed = NO;
        shade.backgroundColor = NSColor.blackColor;
        shade.opaque = YES;
        shade.hasShadow = NO;
        shade.ignoresMouseEvents = YES;
        shade.hidesOnDeactivate = YES;
        shade.level = NSFloatingWindowLevel;
        shade.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
            NSWindowCollectionBehaviorStationary | NSWindowCollectionBehaviorIgnoresCycle;
        [shade orderFront:nil];
        [self.shades addObject:shade];
    }
}
- (void)leave {
    NSWindow *window = self.window;
    if (!window) return;
    self.window = nil;
    [self clearShades];
    NSApp.presentationOptions = self.savedPresentation;
    window.movable = self.savedMovable;
    [window setFrame:self.savedFrame display:YES];
}
- (void)closing:(NSNotification *)notification {
    if (notification.object == self.window) [self leave];
}
@end

static NabiFullscreen *controller;

bool nabi_set_fullscreen(bool enabled, bool blackout) {
    NSCAssert(NSThread.isMainThread, @"Fullscreen must run on the main thread");
    if (!controller) controller = [NabiFullscreen new];
    if (!enabled) { [controller leave]; return false; }
    if (!controller.window) {
        NSWindow *window = NSApp.keyWindow ?: NSApp.mainWindow;
        if (!window || !window.screen) return false;
        controller.window = window;
        controller.savedFrame = window.frame;
        controller.savedMovable = window.movable;
        controller.savedPresentation = NSApp.presentationOptions;
        window.movable = NO;
    } else if (controller.blackout == blackout) {
        return true;
    }
    controller.blackout = blackout;
    [controller refresh:nil];
    return true;
}
