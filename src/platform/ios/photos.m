// PhotoKit read access is requested only when opening a photo bookmark.
// API review: https://developer.apple.com/documentation/photos
#import <Photos/Photos.h>
#import "photo_snapshot.h"
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>
#include <errno.h>

extern const char *nabi_mobile_path(int kind);
extern bool nabi_mobile_import_begin_folder(void);
extern void nabi_mobile_import_source(const char *, const char *);
extern bool nabi_mobile_import_deferred(const char *, const char *, uint64_t, double, int, double);
extern size_t nabi_mobile_import_requested_name(char *, size_t);
extern bool nabi_mobile_import_seed(void);
extern void nabi_mobile_import_finish(void);
extern void nabi_mobile_photo_error(int);
extern void nabi_mobile_import_truncated(void);
extern void nabi_mobile_access_changed(void);

@interface NabiPhotoAccessObserver : NSObject <PHPhotoLibraryChangeObserver>
@end

@implementation NabiPhotoAccessObserver
- (void)photoLibraryDidChange:(PHChange *)changeInstance {
    (void)changeInstance;
    nabi_photo_snapshot_changed();
    // Photos delivers changes on its own queue. Rust only records an event;
    // it decides whether the active source needs refreshing on its UI thread.
    nabi_mobile_access_changed();
}
@end

static void nabi_observe_photo_access(void) {
    static NabiPhotoAccessObserver *observer;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        observer = [NabiPhotoAccessObserver new];
        [PHPhotoLibrary.sharedPhotoLibrary registerChangeObserver:observer];
    });
}

bool nabi_photos_is_collection(NSString *identifier) {
    return [@[@"nabi-ios-library", @"nabi-ios-screenshots", @"nabi-ios-favorites"] containsObject:identifier];
}

int nabi_mobile_photo_status(void) {
    PHAuthorizationStatus status = [PHPhotoLibrary authorizationStatusForAccessLevel:PHAccessLevelReadWrite];
    if (status == PHAuthorizationStatusAuthorized || status == PHAuthorizationStatusLimited) nabi_observe_photo_access();
    switch (status) {
        case PHAuthorizationStatusNotDetermined: return 0;
        case PHAuthorizationStatusRestricted: return 1;
        case PHAuthorizationStatusDenied: return 2;
        case PHAuthorizationStatusAuthorized: return 3;
        case PHAuthorizationStatusLimited: return 4;
    }
    return 1;
}

// The caller lends a temporary output FD for this synchronous request. Late
// callbacks see active=NO under the same lock before Rust closes that FD.
int nabi_photos_read(NSString *token, int fd, uint64_t limit,
                     bool (*live)(void *), void *context) {
    @autoreleasepool {
        if (!live(context)) return -1;
        int status = nabi_mobile_photo_status();
        if (status == 1) return 3;
        if (status != 3 && status != 4) return 1;
        PHAsset *asset = [PHAsset fetchAssetsWithLocalIdentifiers:@[token] options:nil].firstObject;
        if (!asset) return 4;
        PHAssetResource *selected = nil;
        for (PHAssetResource *resource in [PHAssetResource assetResourcesForAsset:asset]) {
            if (resource.type == PHAssetResourceTypeFullSizePhoto) { selected = resource; break; }
            if (resource.type == PHAssetResourceTypePhoto) selected = resource;
        }
        if (!selected) return 5;
        NSLock *lock = [NSLock new];
        dispatch_semaphore_t finished = dispatch_semaphore_create(0);
        __block BOOL active = YES, failed = NO;
        __block int reason = 5;
        __block uint64_t bytes = 0;
        PHAssetResourceRequestOptions *options = [PHAssetResourceRequestOptions new];
        options.networkAccessAllowed = NO;
        PHAssetResourceManager *manager = PHAssetResourceManager.defaultManager;
        PHAssetResourceDataRequestID request = [manager requestDataForAssetResource:selected options:options
            dataReceivedHandler:^(NSData *data) {
                [lock lock];
                if (active && !failed) {
                    if (data.length > limit - bytes) failed = YES;
                    else {
                        const uint8_t *p = data.bytes;
                        NSUInteger left = data.length;
                        while (left) {
                            ssize_t count = write(fd, p, left);
                            if (count < 0 && errno == EINTR) continue;
                            if (count <= 0) { failed = YES; break; }
                            p += count; left -= count; bytes += count;
                        }
                    }
                }
                [lock unlock];
            } completionHandler:^(NSError *error) {
                [lock lock];
                if (error) {
                    failed = YES;
                    if ([error.domain isEqualToString:PHPhotosErrorDomain]) {
                        if (error.code == PHPhotosErrorNetworkAccessRequired) reason = 4;
                        else if (error.code == PHPhotosErrorAccessRestricted) reason = 3;
                        else if (error.code == PHPhotosErrorAccessUserDenied) reason = 1;
                    }
                }
                [lock unlock];
                dispatch_semaphore_signal(finished);
            }];
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:15];
        BOOL completed = NO;
        while (live(context) && deadline.timeIntervalSinceNow > 0) {
            if (dispatch_semaphore_wait(finished, dispatch_time(DISPATCH_TIME_NOW, 50 * NSEC_PER_MSEC)) == 0) {
                completed = YES; break;
            }
            [lock lock];
            BOOL stop = failed;
            [lock unlock];
            if (stop) break;
        }
        BOOL cancelled = !live(context);
        [lock lock];
        active = NO;
        BOOL ok = completed && !cancelled && !failed && bytes > 0;
        int result = cancelled ? -1 : reason;
        [lock unlock];
        if (!ok) [manager cancelDataRequest:request];
        if (ok) {
            NSDate *date = asset.modificationDate ?: asset.creationDate;
            struct timespec times[2] = {{0, 0}, {0, 0}};
            if (date.timeIntervalSince1970 > 0) times[0].tv_sec = times[1].tv_sec = (time_t)date.timeIntervalSince1970;
            futimens(fd, times);
        }
        return ok ? 0 : result;
    }
}

static bool nabi_import_photo_entry(NSDictionary *entry) {
    return nabi_mobile_import_deferred([entry[@"name"] UTF8String], [entry[@"token"] UTF8String],
        0, [entry[@"modified"] doubleValue], [entry[@"resource"] intValue],
        [entry[@"contentModified"] doubleValue]);
}

static void nabi_import_photos(NSString *identifier) {
    @autoreleasepool {
        if (!nabi_mobile_import_begin_folder()) { nabi_mobile_import_finish(); return; }
        nabi_mobile_import_source(identifier.UTF8String, "Photos");
        uint64_t revision = nabi_photo_snapshot_revision();
        int status = nabi_mobile_photo_status();
        bool (^current)(void) = ^bool {
            return (status == 3 || status == 4) && nabi_mobile_photo_status() == status
                && nabi_photo_snapshot_revision() == revision;
        };
        // Names were validated by ImportBatch; still bound the FFI copy and
        // reject an invalid explicit name rather than seeding a different photo.
        char requested[201];
        size_t length = nabi_mobile_import_requested_name(NULL, 0);
        NSString *name = nil;
        BOOL explicitName = length != 0;
        if (length && length <= sizeof(requested)
                && nabi_mobile_import_requested_name(requested, sizeof(requested)) == length
                && requested[length - 1] == 0) {
            name = [NSString stringWithUTF8String:requested];
        }
        PHFetchResult<PHAsset *> *assets = nabi_photo_assets(identifier);
        NSUInteger seedIndex = NSNotFound;
        NSDictionary *seed = (!explicitName || name.length) && current()
            ? nabi_photo_seed_entry(assets, name, &seedIndex, current) : nil;
        BOOL complete = current();
        NSUInteger listed = 0;
        if (seed && complete) {
            // Only this resource has been inspected so far. Publishing does
            // not commit a one-page source marker or finish the import gate.
            complete = nabi_import_photo_entry(seed) && current() && nabi_mobile_import_seed();
            listed++;
        }
        NSMutableArray<NSDictionary *> *entries = [NSMutableArray arrayWithCapacity:assets.count];
        for (NSUInteger i = 0; complete && i < assets.count; i++) {
            @autoreleasepool {
                if (!current()) { complete = NO; break; }
                // Keep the snapshot's native creation-date order, even though
                // the selected file was staged first. Do not read its resource twice.
                NSDictionary *entry = i == seedIndex ? seed : nabi_photo_entry(assets[i]);
                if (!current()) { complete = NO; break; }
                [entries addObject:entry];
                if (i == seedIndex || entry[@"missing"]) continue;
                listed++;
                if (!nabi_import_photo_entry(entry)) {
                    // The 1,001st supported result normally hits Rust's cap.
                    // Other early stops are incomplete/cancelled snapshots.
                    complete = i + 1 == assets.count;
                    break;
                }
                // Unsupported extensions may have skipped the first root
                // candidate. Rust publishes only its first supported seed and
                // treats later calls as no-ops; explicit names never fall back here.
                if (!explicitName && !nabi_mobile_import_seed()) { complete = NO; break; }
            }
        }
        BOOL unchanged = current();
        nabi_photo_snapshot_remember(identifier, entries, revision,
            complete && unchanged && entries.count == assets.count && listed > 0);
        if (!unchanged) {
            int after = nabi_mobile_photo_status();
            nabi_mobile_photo_error(after == 1 ? 3 : (after == 3 || after == 4 ? 5 : 1));
        } else if (!listed) nabi_mobile_photo_error(2);
        nabi_mobile_import_finish();
    }
}

void nabi_photos_open(NSString *identifier, void (^completion)(void)) {
    void (^authorized)(PHAuthorizationStatus) = ^(PHAuthorizationStatus status) {
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
            if (status == PHAuthorizationStatusAuthorized || status == PHAuthorizationStatusLimited) {
                nabi_observe_photo_access();
                nabi_import_photos(identifier);
            } else {
                nabi_mobile_import_begin_folder();
                nabi_mobile_photo_error(status == PHAuthorizationStatusRestricted ? 3 : 1);
                nabi_mobile_import_finish();
            }
            completion();
        });
    };
    PHAuthorizationStatus status = [PHPhotoLibrary authorizationStatusForAccessLevel:PHAccessLevelReadWrite];
    if (status == PHAuthorizationStatusNotDetermined) {
        [PHPhotoLibrary requestAuthorizationForAccessLevel:PHAccessLevelReadWrite handler:authorized];
    } else authorized(status);
}
