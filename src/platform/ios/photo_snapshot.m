// PhotoKit-specific metadata snapshots. No original pixels or permission grants are cached.
// API review 2026-11-16: https://developer.apple.com/documentation/photos/phasset
#import "photo_snapshot.h"
#import <Photos/Photos.h>
extern int nabi_mobile_photo_status(void);
extern void nabi_mobile_photo_validation_result(uint64_t, bool);

// One bounded metadata snapshot for the batch currently owned by the viewer.
// The lock never covers PhotoKit queries or calls into Rust.
static NSObject *nabi_photo_snapshot_lock(void) {
    static NSObject *lock;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ lock = [NSObject new]; });
    return lock;
}
static NSString *nabi_snapshot_source;
static NSArray<NSDictionary *> *nabi_snapshot_entries;
static uint64_t nabi_photo_changes;
static uint64_t nabi_latest_validation;

static dispatch_queue_t nabi_photo_validation_queue(void) {
    static dispatch_queue_t queue;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        dispatch_queue_attr_t attributes = dispatch_queue_attr_make_with_qos_class(
            DISPATCH_QUEUE_SERIAL, QOS_CLASS_USER_INITIATED, 0);
        queue = dispatch_queue_create("nabi.photos.validation", attributes);
    });
    return queue;
}

void nabi_photo_snapshot_changed(void) {
    @synchronized (nabi_photo_snapshot_lock()) { nabi_photo_changes++; }
}

uint64_t nabi_photo_snapshot_revision(void) {
    @synchronized (nabi_photo_snapshot_lock()) { return nabi_photo_changes; }
}

void nabi_photo_snapshot_remember(NSString *identifier, NSArray<NSDictionary *> *entries,
                                   uint64_t revision, bool complete) {
    @synchronized (nabi_photo_snapshot_lock()) {
        if (complete && revision == nabi_photo_changes) {
            nabi_snapshot_source = [identifier copy];
            nabi_snapshot_entries = entries;
        } else {
            nabi_snapshot_source = nil;
            nabi_snapshot_entries = nil;
        }
    }
}

// PHFetchResult is lazy. Returning it lets a requested page reach the renderer
// before the rest of the collection's resource metadata has been inspected.
PHFetchResult<PHAsset *> *nabi_photo_assets(NSString *identifier) {
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
    return [PHAsset fetchAssetsWithOptions:options];
}

NSDictionary *nabi_photo_entry(PHAsset *asset) {
    PHAssetResource *selected = nil;
    for (PHAssetResource *resource in [PHAssetResource assetResourcesForAsset:asset]) {
        if (resource.type == PHAssetResourceTypeFullSizePhoto) { selected = resource; break; }
        if (resource.type == PHAssetResourceTypePhoto) selected = resource;
    }
    if (!selected) return @{@"token": asset.localIdentifier, @"missing": @YES};
    NSString *key = [asset.localIdentifier componentsSeparatedByString:@"/"].firstObject;
    NSString *name = [NSString stringWithFormat:@"%@-%@", key, selected.originalFilename];
    NSDate *date = asset.modificationDate ?: asset.creationDate;
    // PhotoKit exposes pixel dimensions but no public original-byte count.
    // No original is opened, downloaded or copied while building metadata.
    return @{
        @"name": name, @"token": asset.localIdentifier,
        @"modified": @(date.timeIntervalSince1970),
        @"contentModified": @(asset.modificationDate.timeIntervalSince1970),
        @"created": @(asset.creationDate.timeIntervalSince1970),
        @"width": @(asset.pixelWidth), @"height": @(asset.pixelHeight),
        @"subtypes": @(asset.mediaSubtypes), @"favorite": @(asset.favorite),
        @"resource": @(selected.type)
    };
}

NSDictionary *nabi_photo_seed_entry(PHFetchResult<PHAsset *> *assets, NSString *name,
                                    NSUInteger *index, bool (^current)(void)) {
    *index = NSNotFound;
    NSUInteger count = MIN((NSUInteger)1000, assets.count);
    for (NSUInteger i = 0; i < count; i++) {
        @autoreleasepool {
            if (!current()) return nil;
            PHAsset *asset = assets[i];
            if (name) {
                // Do not reconstruct PhotoKit's opaque localIdentifier from a
                // filename. Compare its existing filename prefix to avoid doing
                // resource I/O for every other asset in the lazy result.
                NSString *key = [asset.localIdentifier componentsSeparatedByString:@"/"].firstObject;
                if (![name hasPrefix:[key stringByAppendingString:@"-"]]) continue;
            }
            NSDictionary *entry = nabi_photo_entry(asset);
            if (!current()) return nil;
            if (entry[@"missing"] || (name && ![entry[@"name"] isEqualToString:name])) continue;
            *index = i;
            return entry;
        }
    }
    return nil;
}

// Resume validation still compares the complete bounded metadata snapshot.
NSArray<NSDictionary *> *nabi_photo_entries(NSString *identifier) {
    @autoreleasepool {
        PHFetchResult<PHAsset *> *assets = nabi_photo_assets(identifier);
        NSMutableArray<NSDictionary *> *entries = [NSMutableArray arrayWithCapacity:assets.count];
        for (PHAsset *asset in assets) {
            @autoreleasepool {
                [entries addObject:nabi_photo_entry(asset)];
            }
        }
        return [entries copy];
    }
}

// Resume is a request to revalidate, not evidence that the collection changed.
// Even Limited -> Limited must compare the actual authorized identifier set.
void nabi_photos_validate(const char *source, uint64_t request) {
    NSString *identifier = source ? [NSString stringWithUTF8String:source] : nil;
    @synchronized (nabi_photo_snapshot_lock()) { nabi_latest_validation = request; }
    dispatch_async(nabi_photo_validation_queue(), ^{
        @autoreleasepool {
            @synchronized (nabi_photo_snapshot_lock()) {
                if (request != nabi_latest_validation) return;
            }
            BOOL same = NO;
            int status = nabi_mobile_photo_status();
            if (identifier && (status == 3 || status == 4)) {
                NSArray<NSDictionary *> *previous;
                uint64_t revision;
                @synchronized (nabi_photo_snapshot_lock()) {
                    previous = [nabi_snapshot_source isEqualToString:identifier] ? nabi_snapshot_entries : nil;
                    revision = nabi_photo_changes;
                }
                if (previous) {
                    NSArray<NSDictionary *> *current = nabi_photo_entries(identifier);
                    int after = nabi_mobile_photo_status();
                    @synchronized (nabi_photo_snapshot_lock()) {
                        same = status == after && revision == nabi_photo_changes
                            && previous == nabi_snapshot_entries
                            && [previous isEqualToArray:current];
                    }
                }
            }
            @synchronized (nabi_photo_snapshot_lock()) {
                if (request != nabi_latest_validation) return;
            }
            nabi_mobile_photo_validation_result(request, same);
        }
    });
}

