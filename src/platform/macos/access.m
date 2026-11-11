#import <AppKit/AppKit.h>
#import <Security/Security.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>

// This registry belongs to app state, never portable settings or image caches.
// One lock covers each read/merge/atomic-write across independent viewer windows.
@class NabiReadLease;
static NSString *registryPath;
static NSString *leaseDirectory;
extern int nabi_access_referenced(const char *original, const char *current);
extern void nabi_access_references_refresh(void);
static NSMutableArray<NSURL *> *pendingURLs;
static NSMutableDictionary<NSString *, NabiReadLease *> *temporaryLeases;
static const NSUInteger maxRegistryBytes = 8 * 1024 * 1024;
static const NSUInteger maxBookmarks = 1024;

@interface NabiReadLease : NSObject
@property NSURL *url;
@property BOOL started;
@property int grantLock;
@end
@implementation NabiReadLease
- (instancetype)init { self = [super init]; if (self) _grantLock = -1; return self; }
- (void)dealloc {
    if (_started) [_url stopAccessingSecurityScopedResource];
    if (_grantLock >= 0) { flock(_grantLock, LOCK_UN); close(_grantLock); }
}
@end

static BOOL sandboxed(void) {
    static BOOL result;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        SecTaskRef task = SecTaskCreateFromSelf(kCFAllocatorDefault);
        if (task) {
            CFTypeRef value = SecTaskCopyValueForEntitlement(task, CFSTR("com.apple.security.app-sandbox"), NULL);
            result = value && CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value);
            if (value) CFRelease(value);
            CFRelease(task);
        }
    });
    return result;
}
static NSURL *fileURL(const char *path) {
    if (!path) return nil;
    return [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
}
static BOOL beneath(NSString *path, NSString *root) {
    if ([root isEqualToString:@"/"]) return [path hasPrefix:@"/"];
    return [path isEqualToString:root] || [path hasPrefix:[root stringByAppendingString:@"/"]];
}
static BOOL readable(NSURL *url) {
    return url.fileSystemRepresentation && access(url.fileSystemRepresentation, R_OK) == 0;
}
static NSString *currentPath(NSURL *url) {
    return url.URLByResolvingSymlinksInPath.path.stringByStandardizingPath;
}
static void fail(char **error, NSString *message) {
    if (error) *error = strdup((message ?: @"Could not access this document. Choose it again.").UTF8String);
}
static int lockRegistry(char **error) {
    if (!registryPath) return -1;
    NSString *lock = [registryPath stringByAppendingString:@".lock"];
    int descriptor = open(lock.fileSystemRepresentation, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (descriptor < 0 || flock(descriptor, LOCK_EX) != 0) {
        if (descriptor >= 0) close(descriptor);
        fail(error, @"Could not lock saved document permissions.");
        return -1;
    }
    return descriptor;
}
static NSMutableDictionary *loadRegistry(char **error) {
    NSDictionary *attributes = [[NSFileManager defaultManager] attributesOfItemAtPath:registryPath error:nil];
    if (!attributes) return [NSMutableDictionary dictionary];
    if (![attributes[NSFileType] isEqual:NSFileTypeRegular] || [attributes[NSFileSize] unsignedLongLongValue] > maxRegistryBytes) {
        fail(error, @"Saved document permissions exceed the storage limit."); return nil;
    }
    NSData *data = [NSData dataWithContentsOfFile:registryPath];
    id value = data ? [NSPropertyListSerialization propertyListWithData:data options:NSPropertyListMutableContainers format:nil error:nil] : nil;
    if (![value isKindOfClass:NSMutableDictionary.class] || [value count] > maxBookmarks) {
        fail(error, @"Saved document permissions could not be read. Choose the document again."); return nil;
    }
    for (id key in value) {
        NSMutableDictionary *row = value[key];
        if (![key isKindOfClass:NSString.class] || ![key hasPrefix:@"/"] || ![row isKindOfClass:NSDictionary.class]
            || ![row[@"bookmark"] isKindOfClass:NSData.class] || [row[@"bookmark"] length] > 1024 * 1024
            || ![row[@"resolved"] isKindOfClass:NSString.class] || ![row[@"resolved"] hasPrefix:@"/"]
            || ![row[@"directory"] isKindOfClass:NSNumber.class]) {
            fail(error, @"Invalid saved document permission entry."); return nil;
        }
        if (![row[@"id"] isKindOfClass:NSString.class] || ![[NSUUID alloc] initWithUUIDString:row[@"id"]]) row[@"id"] = NSUUID.UUID.UUIDString;
        if (![row[@"used"] isKindOfClass:NSNumber.class]) row[@"used"] = @0;
    }
    return value;
}
static int lockGrant(NSDictionary *record, int mode) {
    NSString *identifier = record[@"id"];
    if (!identifier || !leaseDirectory) return -1;
    NSString *path = [leaseDirectory stringByAppendingPathComponent:[identifier stringByAppendingString:@".lock"]];
    int descriptor = open(path.fileSystemRepresentation, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (descriptor >= 0 && flock(descriptor, mode | LOCK_NB) != 0) { close(descriptor); return -1; }
    return descriptor;
}
static BOOL registryFits(NSDictionary *entries) {
    if (entries.count > maxBookmarks) return NO;
    NSData *data = [NSPropertyListSerialization dataWithPropertyList:entries format:NSPropertyListBinaryFormat_v1_0 options:0 error:nil];
    return data && data.length <= maxRegistryBytes;
}
static void cleanupGrantLocks(NSDictionary *entries) {
    NSMutableSet *used = [NSMutableSet set];
    for (NSDictionary *record in entries.allValues) if (record[@"id"]) [used addObject:record[@"id"]];
    for (NSString *name in [[NSFileManager defaultManager] contentsOfDirectoryAtPath:leaseDirectory error:nil]) {
        if (![name.pathExtension isEqualToString:@"lock"]) continue;
        NSString *identifier = name.stringByDeletingPathExtension;
        if (![[NSUUID alloc] initWithUUIDString:identifier] || [used containsObject:identifier]) continue;
        NSString *path = [leaseDirectory stringByAppendingPathComponent:name];
        int descriptor = open(path.fileSystemRepresentation, O_RDWR | O_CLOEXEC | O_NOFOLLOW);
        if (descriptor >= 0) {
            if (flock(descriptor, LOCK_EX | LOCK_NB) == 0) { unlink(path.fileSystemRepresentation); flock(descriptor, LOCK_UN); }
            close(descriptor);
        }
    }
}
static void trimRegistry(NSMutableDictionary *entries, NSString *selected) {
    if (registryFits(entries)) return;
    nabi_access_references_refresh(); // One bounded SQLite read per trim, not per row.
    NSArray *oldest = [entries.allKeys sortedArrayUsingComparator:^NSComparisonResult(NSString *a, NSString *b) {
        return [entries[a][@"used"] compare:entries[b][@"used"]];
    }];
    for (NSString *key in oldest) {
        if ([key isEqualToString:selected]) continue;
        NSDictionary *record = entries[key];
        // Only raw, read-only SQLite is consulted by this Rust callback. It
        // never calls back into the registry; failure conservatively retains.
        if (nabi_access_referenced(key.fileSystemRepresentation, [record[@"resolved"] fileSystemRepresentation])) continue;
        int active = lockGrant(record, LOCK_EX);
        if (active < 0) continue; // An active viewer/worker in any process owns it.
        [entries removeObjectForKey:key];
        flock(active, LOCK_UN); close(active);
        if (registryFits(entries)) break;
    }
}
static BOOL saveRegistry(NSDictionary *entries, char **error) {
    NSError *cause = nil;
    NSData *data = [NSPropertyListSerialization dataWithPropertyList:entries format:NSPropertyListBinaryFormat_v1_0 options:0 error:&cause];
    if (!data || data.length > maxRegistryBytes || entries.count > maxBookmarks || ![data writeToFile:registryPath options:NSDataWritingAtomic error:&cause]) {
        fail(error, cause ? [@"Could not save document permissions: " stringByAppendingString:cause.localizedDescription] : @"Saved document permissions exceed the storage limit."); return NO;
    }
    chmod(registryPath.fileSystemRepresentation, 0600);
    return YES;
}
static NSData *bookmark(NSURL *url) {
    // Unsandboxed builds need file identity only, never a transferable ephemeral
    // sandbox grant. Store builds persist the narrow, app-scoped read permission.
    NSURLBookmarkCreationOptions options = sandboxed()
        ? NSURLBookmarkCreationWithSecurityScope | NSURLBookmarkCreationSecurityScopeAllowOnlyReadAccess
        : NSURLBookmarkCreationWithoutImplicitSecurityScope;
    return [url bookmarkDataWithOptions:options includingResourceValuesForKeys:@[NSURLIsDirectoryKey] relativeToURL:nil error:nil];
}
static BOOL rememberURL(NSURL *url, char **error) {
    if (!url.isFileURL || !registryPath) return NO;
    NabiReadLease *lease = [NabiReadLease new];
    lease.url = url; lease.started = [url startAccessingSecurityScopedResource];
    if (!readable(url)) { fail(error, @"Permission is unavailable. Choose the file or folder again."); return NO; }
    NSData *data = bookmark(url);
    if (!data || data.length > 1024 * 1024) { fail(error, @"This selection cannot retain permission after the app closes."); return NO; }
    int lock = lockRegistry(error); if (lock < 0) return NO;
    NSMutableDictionary *entries = loadRegistry(error);
    BOOL saved = NO;
    if (entries) {
        NSString *path = url.path.stringByStandardizingPath;
        NSNumber *directory = nil; [url getResourceValue:&directory forKey:NSURLIsDirectoryKey error:nil];
        entries[path] = [@{@"bookmark": data, @"resolved": currentPath(url), @"directory": directory ?: @NO,
            @"id": entries[path][@"id"] ?: NSUUID.UUID.UUIDString,
            @"used": @([NSDate timeIntervalSinceReferenceDate])} mutableCopy];
        trimRegistry(entries, path);
        saved = saveRegistry(entries, error);
        if (saved) cleanupGrantLocks(entries);
    }
    flock(lock, LOCK_UN); close(lock);
    return saved;
}
static void holdTemporary(NSURL *url) {
    if (!temporaryLeases) temporaryLeases = [NSMutableDictionary dictionary];
    if (temporaryLeases.count >= maxBookmarks) [temporaryLeases removeObjectForKey:temporaryLeases.allKeys.firstObject];
    NabiReadLease *lease = [NabiReadLease new]; lease.url = url;
    lease.started = [url startAccessingSecurityScopedResource];
    temporaryLeases[url.path.stringByStandardizingPath] = lease;
}

int nabi_access_configure(const char *directory, char **error) {
    @autoreleasepool { @synchronized(NabiReadLease.class) {
        NSURL *root = fileURL(directory);
        if (!root || ![[NSFileManager defaultManager] createDirectoryAtURL:root withIntermediateDirectories:YES attributes:@{NSFilePosixPermissions:@0700} error:nil]) {
            fail(error, @"Document permission storage is unavailable."); return 1;
        }
        registryPath = [root.path stringByAppendingPathComponent:@"macos-bookmarks.plist"];
        leaseDirectory = [root.path stringByAppendingPathComponent:@"macos-grant-locks"];
        if (![[NSFileManager defaultManager] createDirectoryAtPath:leaseDirectory withIntermediateDirectories:YES attributes:@{NSFilePosixPermissions:@0700} error:nil]) {
            fail(error, @"Document permission lifetime storage is unavailable."); return 1;
        }
        for (NSURL *url in pendingURLs) if (!rememberURL(url, NULL)) holdTemporary(url);
        pendingURLs = nil;
        int lock = lockRegistry(NULL);
        NSDictionary *entries = lock >= 0 ? loadRegistry(NULL) : nil;
        if (entries) cleanupGrantLocks(entries);
        if (lock >= 0) { flock(lock, LOCK_UN); close(lock); }
        return 0;
    }}
}
void nabi_access_capture_url(NSURL *url) {
    @autoreleasepool { @synchronized(NabiReadLease.class) {
        if (!registryPath) {
            if (!pendingURLs) pendingURLs = [NSMutableArray array];
            if (pendingURLs.count < maxBookmarks) [pendingURLs addObject:url];
        } else if (!rememberURL(url, NULL)) holdTemporary(url);
    }}
}
int nabi_access_remember(const char *path, char **error) {
    @autoreleasepool { @synchronized(NabiReadLease.class) { return rememberURL(fileURL(path), error) ? 0 : 1; }}
}
// Keep the original Powerbox NSURL until its read bookmark has been created;
// reducing it to a plain path first can discard URL-specific scope information.
int nabi_access_select(const char *initial, const char *extensions, bool folder,
                       char ***paths, void ***leases, size_t *count, char **error) {
    @autoreleasepool {
        if (![NSThread isMainThread] || !paths || !leases || !count) return 1;
        *paths = NULL; *leases = NULL; *count = 0;
        NSOpenPanel *panel = [NSOpenPanel openPanel];
        panel.canChooseDirectories = folder; panel.canChooseFiles = !folder;
        panel.allowsMultipleSelection = !folder;
        if (initial) {
            NSURL *url = fileURL(initial);
            NSNumber *directory = nil; [url getResourceValue:&directory forKey:NSURLIsDirectoryKey error:nil];
            panel.directoryURL = directory.boolValue ? url : url.URLByDeletingLastPathComponent;
            if (!directory.boolValue) panel.nameFieldStringValue = url.lastPathComponent;
        }
        if (!folder && extensions) {
            NSMutableArray<UTType *> *types = [NSMutableArray array];
            for (NSString *ext in [[NSString stringWithUTF8String:extensions] componentsSeparatedByString:@","]) {
                UTType *type = [UTType typeWithFilenameExtension:ext];
                if (type) [types addObject:type];
            }
            if (types.count) panel.allowedContentTypes = types;
        }
        if ([panel runModal] != NSModalResponseOK) return 2;
        NSArray<NSURL *> *urls = panel.URLs;
        if (!urls.count || urls.count > maxBookmarks) { fail(error, @"Choose at most 1,024 documents."); return 1; }
        char **result = calloc(urls.count, sizeof(char *));
        void **handles = calloc(urls.count, sizeof(void *));
        if (!result || !handles) { free(result); free(handles); return 1; }
        NSUInteger copied = 0;
        for (NSURL *url in urls) {
            NabiReadLease *lease = [NabiReadLease new]; lease.url = url;
            lease.started = [url startAccessingSecurityScopedResource];
            @synchronized(NabiReadLease.class) {
                // A persistence failure is reported, but does not discard a
                // user's readable selection from the current session.
                char *warning = NULL;
                rememberURL(url, &warning);
                if (warning) { if (error && !*error) *error = warning; else free(warning); }
                int lock = lockRegistry(NULL);
                NSDictionary *entries = lock >= 0 ? loadRegistry(NULL) : nil;
                if (entries[url.path.stringByStandardizingPath]) lease.grantLock = lockGrant(entries[url.path.stringByStandardizingPath], LOCK_SH);
                if (lock >= 0) { flock(lock, LOCK_UN); close(lock); }
            }
            result[copied] = strdup(url.fileSystemRepresentation);
            if (!result[copied]) {
                for (NSUInteger i = 0; i < copied; i++) { free(result[i]); CFRelease(handles[i]); }
                free(result); free(handles); return 1;
            }
            handles[copied] = (__bridge_retained void *)lease;
            copied++;
        }
        *paths = result; *leases = handles; *count = copied;
        return 0;
    }
}
int nabi_access_has(const char *path) {
    @autoreleasepool { @synchronized(NabiReadLease.class) {
        if (!registryPath) return 0;
        NSString *candidate = fileURL(path).path.stringByStandardizingPath;
        int lock = lockRegistry(NULL); if (lock < 0) return 0;
        NSDictionary *entries = loadRegistry(NULL);
        BOOL found = NO;
        for (NSString *key in entries) {
            if (beneath(candidate, key) || beneath(candidate, entries[key][@"resolved"]) || beneath(key, candidate)) { found = YES; break; }
        }
        flock(lock, LOCK_UN); close(lock);
        return found;
    }}
}
bool nabi_access_temporary(const char *path) {
    @autoreleasepool { @synchronized(NabiReadLease.class) {
        return temporaryLeases[fileURL(path).path.stringByStandardizingPath] != nil;
    }}
}
int nabi_access_resolve(const char *path, char **resolved, void **handle, char **error) {
    @autoreleasepool { @synchronized(NabiReadLease.class) {
        if (!resolved || !handle || !path) return 1;
        *resolved = NULL; *handle = NULL;
        NSURL *requested = fileURL(path);
        if (!requested) return 1;
        NSString *wanted = requested.path.stringByStandardizingPath;
        int lock = registryPath ? lockRegistry(error) : -1;
        NSMutableDictionary *entries = lock >= 0 ? loadRegistry(error) : nil;
        NSString *selected = nil, *matched = nil;
        for (NSString *key in entries) {
            for (NSString *prefix in @[key, entries[key][@"resolved"]]) {
                if (beneath(wanted, prefix) && (!matched || prefix.length > matched.length)) {
                    selected = key; matched = prefix;
                }
            }
        }
        NabiReadLease *temporary = temporaryLeases[wanted];
        NSURL *url = temporary ? temporary.url : requested;
        BOOL stale = NO;
        if (selected) {
            NSURLBookmarkResolutionOptions options = NSURLBookmarkResolutionWithoutUI | NSURLBookmarkResolutionWithoutMounting;
            if (sandboxed()) options |= NSURLBookmarkResolutionWithSecurityScope;
            url = [NSURL URLByResolvingBookmarkData:entries[selected][@"bookmark"] options:options relativeToURL:nil bookmarkDataIsStale:&stale error:nil];
        }
        NabiReadLease *lease = [NabiReadLease new];
        lease.url = url; lease.started = url && [url startAccessingSecurityScopedResource];
        if (!url || (sandboxed() && !readable(url)) || (selected && !readable(url))) {
            if (lock >= 0) { flock(lock, LOCK_UN); close(lock); }
            fail(error, @"Access to this file or folder was lost. Choose it again to restore access."); return 1;
        }
        NSString *destination = wanted;
        if (selected) {
            lease.grantLock = lockGrant(entries[selected], LOCK_SH);
            NSString *suffix = [wanted substringFromIndex:matched.length];
            destination = [currentPath(url) stringByAppendingString:suffix];
            if (stale || ![currentPath(url) isEqualToString:entries[selected][@"resolved"]]) {
                NSData *data = bookmark(url);
                if (data) {
                    NSMutableDictionary *record = [entries[selected] mutableCopy];
                    record[@"bookmark"] = data; record[@"resolved"] = currentPath(url);
                    entries[selected] = record;
                    saveRegistry(entries, NULL); // A read grant remains usable if its refreshed storage cannot be written.
                }
            }
            double now = [NSDate timeIntervalSinceReferenceDate];
            if (now - [entries[selected][@"used"] doubleValue] > 60) {
                entries[selected][@"used"] = @(now);
                saveRegistry(entries, NULL);
            }
        }
        if (lock >= 0) { flock(lock, LOCK_UN); close(lock); }
        *resolved = strdup([[NSFileManager defaultManager] fileSystemRepresentationWithPath:destination]);
        *handle = (__bridge_retained void *)lease;
        if (temporary) [temporaryLeases removeObjectForKey:wanted];
        return *resolved ? 0 : 1;
    }}
}
// Explicit UI recovery. A cancellation returns no selection and changes no grant.
int nabi_access_pick(const char *previous, bool folder, char **selected, char **error) {
    @autoreleasepool {
        if (![NSThread isMainThread] || !selected) return 1;
        *selected = NULL;
        NSString *wanted = fileURL(previous).path.stringByStandardizingPath;
        __block NSString *matched = nil, *key = nil, *suffix = @"";
        @synchronized(NabiReadLease.class) {
            int lock = lockRegistry(NULL);
            NSDictionary *entries = lock >= 0 ? loadRegistry(NULL) : nil;
            for (NSString *candidate in entries) {
                for (NSString *prefix in @[candidate, entries[candidate][@"resolved"]]) {
                    if (beneath(wanted, prefix) && (!matched || prefix.length > matched.length)) {
                        key = candidate; matched = prefix; folder = [entries[candidate][@"directory"] boolValue];
                    }
                }
            }
            if (matched) suffix = [wanted substringFromIndex:matched.length];
            if (lock >= 0) { flock(lock, LOCK_UN); close(lock); }
        }
        NSOpenPanel *panel = [NSOpenPanel openPanel];
        panel.canChooseDirectories = folder; panel.canChooseFiles = !folder;
        panel.allowsMultipleSelection = NO;
        panel.message = @"Choose the file or folder again to restore access. Cancel keeps the current image and saved entry.";
        NSURL *hint = matched ? [NSURL fileURLWithPath:matched] : fileURL(previous);
        panel.directoryURL = folder ? hint : hint.URLByDeletingLastPathComponent;
        if ([panel runModal] != NSModalResponseOK) return 2;
        NSURL *url = panel.URL;
        @synchronized(NabiReadLease.class) {
            if (!rememberURL(url, error)) return 1;
            if (key) {
                // Keep the old library identity as an alias to the chosen root.
                int lock = lockRegistry(error); if (lock < 0) return 1;
                NSMutableDictionary *entries = loadRegistry(error);
                NSDictionary *replacement = entries[url.path.stringByStandardizingPath];
                BOOL saved = NO;
                if (entries && replacement) { entries[key] = replacement; saved = saveRegistry(entries, error); }
                flock(lock, LOCK_UN); close(lock);
                if (!saved) return 1;
            }
        }
        NSString *destination = [url.path.stringByStandardizingPath stringByAppendingString:suffix];
        *selected = strdup([[NSFileManager defaultManager] fileSystemRepresentationWithPath:destination]);
        return *selected ? 0 : 1;
    }
}
// Read bookmarks never silently become persistent write grants. A user-initiated
// move/trash can explicitly select its parent folder for a temporary write scope.
int nabi_access_write(const char *path, void **handle, char **error) {
    @autoreleasepool {
        if (!handle || !path || ![NSThread isMainThread]) return 1;
        *handle = NULL;
        NSURL *url = fileURL(path);
        if (!sandboxed() || access(path, W_OK) == 0) return 0;
        NSOpenPanel *panel = [NSOpenPanel openPanel];
        panel.canChooseFiles = NO; panel.canChooseDirectories = YES; panel.allowsMultipleSelection = NO;
        panel.directoryURL = url;
        panel.message = @"Choose this folder to allow the file operation you requested. Reading photos keeps read-only saved access.";
        if ([panel runModal] != NSModalResponseOK) return 2;
        NSURL *chosen = panel.URL.URLByResolvingSymlinksInPath;
        if (![chosen.path.stringByStandardizingPath isEqualToString:url.URLByResolvingSymlinksInPath.path.stringByStandardizingPath]) {
            fail(error, @"Choose the exact source or destination folder for this operation."); return 1;
        }
        NabiReadLease *lease = [NabiReadLease new]; lease.url = panel.URL;
        lease.started = [lease.url startAccessingSecurityScopedResource];
        if (access(path, W_OK) != 0) { fail(error, @"The selected folder does not allow this file operation."); return 1; }
        *handle = (__bridge_retained void *)lease;
        return 0;
    }
}
void nabi_access_release(void *handle) { if (handle) CFRelease(handle); }
void nabi_access_free(void *value) { free(value); }
bool nabi_access_is_sandboxed(void) { return sandboxed(); }
int nabi_access_forget(const char *removed, const char *const *retained, size_t count, char **error) {
    @autoreleasepool { @synchronized(NabiReadLease.class) {
        if (!registryPath || !removed || count > 100000) return 0;
        NSString *root = fileURL(removed).path.stringByStandardizingPath;
        NSMutableArray<NSString *> *keep = [NSMutableArray array];
        for (size_t index = 0; index < count; index++) {
            NSString *path = fileURL(retained[index]).path.stringByStandardizingPath;
            if (path) [keep addObject:path];
        }
        int lock = lockRegistry(error); if (lock < 0) return 1;
        NSMutableDictionary *entries = loadRegistry(error);
        for (NSString *key in entries.allKeys) {
            NSString *current = entries[key][@"resolved"];
            if (!beneath(key, root) && !beneath(root, key) && !beneath(current, root) && !beneath(root, current)) continue;
            BOOL used = NO;
            for (NSString *path in keep) {
                if (beneath(path, key) || beneath(key, path) || beneath(path, current) || beneath(current, path)) { used = YES; break; }
            }
            if (!used) [entries removeObjectForKey:key];
        }
        BOOL saved = entries && saveRegistry(entries, error);
        if (saved) cleanupGrantLocks(entries);
        flock(lock, LOCK_UN); close(lock);
        return saved ? 0 : 1;
    }}
}
