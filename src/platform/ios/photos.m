// PhotoKit read access is requested only when opening a photo bookmark.
// API review: https://developer.apple.com/documentation/photos
#import <Photos/Photos.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>

extern const char *nabi_mobile_path(int kind);
extern bool nabi_mobile_import_begin_folder(void);
extern void nabi_mobile_import_source(const char *, const char *);
extern bool nabi_mobile_import_file(int, const char *);
extern void nabi_mobile_import_finish(void);
extern void nabi_mobile_photo_error(int);
extern void nabi_mobile_import_truncated(void);
extern void nabi_mobile_access_changed(void);

@interface NabiPhotoAccessObserver : NSObject <PHPhotoLibraryChangeObserver>
@end

@implementation NabiPhotoAccessObserver
- (void)photoLibraryDidChange:(PHChange *)changeInstance {
    (void)changeInstance;
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

// Stream one local resource into an unlinked cache file. Memory, bytes and time
// are bounded even when PhotoKit calls back after a timeout/cancellation.
static int nabi_photo_file(PHAsset *asset, PHAssetResource *resource, int *failure) {
    *failure = 5; // Preserve unknown I/O failures instead of calling them denial.
    NSString *template = [[NSString stringWithUTF8String:nabi_mobile_path(1)] stringByAppendingPathComponent:@"photo-XXXXXX"];
    char *path = strdup(template.fileSystemRepresentation);
    int fd = mkstemp(path);
    if (fd >= 0) unlink(path);
    free(path);
    if (fd < 0) return -1;
    NSLock *lock = [NSLock new];
    dispatch_semaphore_t finished = dispatch_semaphore_create(0);
    __block BOOL active = YES, failed = NO;
    __block int reason = 5;
    __block uint64_t bytes = 0;
    PHAssetResourceRequestOptions *options = [PHAssetResourceRequestOptions new];
    options.networkAccessAllowed = NO;
    PHAssetResourceManager *manager = PHAssetResourceManager.defaultManager;
    PHAssetResourceDataRequestID request = [manager requestDataForAssetResource:resource options:options
        dataReceivedHandler:^(NSData *data) {
            [lock lock];
            if (active && !failed) {
                if (data.length > 256ULL * 1024 * 1024 - bytes) failed = YES;
                else {
                    const uint8_t *p = data.bytes;
                    NSUInteger left = data.length;
                    while (left) {
                        ssize_t count = write(fd, p, left);
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
    BOOL timedOut = dispatch_semaphore_wait(finished, dispatch_time(DISPATCH_TIME_NOW, 15 * NSEC_PER_SEC)) != 0;
    [lock lock];
    active = NO;
    BOOL ok = !timedOut && !failed && bytes > 0;
    *failure = timedOut ? 5 : reason;
    [lock unlock];
    if (timedOut || !ok) [manager cancelDataRequest:request];
    if (!ok || lseek(fd, 0, SEEK_SET) < 0) { close(fd); return -1; }
    NSDate *date = asset.modificationDate ?: asset.creationDate;
    struct timespec times[2] = {{0, 0}, {0, 0}};
    if (date.timeIntervalSince1970 > 0) times[0].tv_sec = times[1].tv_sec = (time_t)date.timeIntervalSince1970;
    futimens(fd, times);
    return fd;
}

static void nabi_import_photos(NSString *identifier) {
    @autoreleasepool {
        if (!nabi_mobile_import_begin_folder()) { nabi_mobile_import_finish(); return; }
        nabi_mobile_import_source(identifier.UTF8String, "Photos");
        PHFetchOptions *options = [PHFetchOptions new];
        NSPredicate *images = [NSPredicate predicateWithFormat:@"mediaType == %d", PHAssetMediaTypeImage];
        NSPredicate *collection = nil;
        if ([identifier isEqualToString:@"nabi-ios-screenshots"]) {
            collection = [NSPredicate predicateWithFormat:@"(mediaSubtypes & %llu) != 0", (unsigned long long)PHAssetMediaSubtypePhotoScreenshot];
        } else if ([identifier isEqualToString:@"nabi-ios-favorites"]) {
            collection = [NSPredicate predicateWithFormat:@"favorite == YES"];
        }
        // Filter the authorized asset set directly. Limited access need not
        // expose smart-album collections even when it exposes matching photos.
        options.predicate = collection ? [NSCompoundPredicate andPredicateWithSubpredicates:@[images, collection]] : images;
        options.sortDescriptors = @[[NSSortDescriptor sortDescriptorWithKey:@"creationDate" ascending:NO]];
        // The shared import layer limits collections to 1,000 images / 2 GiB.
        // Fetch one extra so that it can report truncation.
        options.fetchLimit = 1001;
        PHFetchResult<PHAsset *> *assets = [PHAsset fetchAssetsWithOptions:options];
        NSUInteger imported = 0;
        int failure = assets.count ? 4 : 2;
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:60];
        for (PHAsset *asset in assets) {
            @autoreleasepool {
                if (deadline.timeIntervalSinceNow <= 0) { failure = 5; nabi_mobile_import_truncated(); break; }
                PHAssetResource *selected = nil;
                for (PHAssetResource *resource in [PHAssetResource assetResourcesForAsset:asset]) {
                    if (resource.type == PHAssetResourceTypeFullSizePhoto) { selected = resource; break; }
                    if (resource.type == PHAssetResourceTypePhoto) selected = resource;
                }
                if (!selected) { failure = 5; continue; }
                int reason = 5;
                int fd = nabi_photo_file(asset, selected, &reason);
                if (fd < 0) {
                    // Only exclusively network-required failures use the local
                    // originals message. Other errors retain their own reason.
                    if (reason != 4) failure = reason;
                    if (reason == 1 || reason == 3) {
                        // Revocation invalidates an in-progress batch even if
                        // earlier resources were already copied successfully.
                        nabi_mobile_photo_error(reason);
                        break;
                    }
                    continue;
                }
                NSString *key = [asset.localIdentifier componentsSeparatedByString:@"/"].firstObject;
                NSString *name = [NSString stringWithFormat:@"%@-%@", key, selected.originalFilename];
                imported++;
                if (!nabi_mobile_import_file(fd, name.UTF8String)) break;
            }
        }
        if (!imported) nabi_mobile_photo_error(failure);
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
