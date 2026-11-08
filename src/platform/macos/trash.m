#import <Foundation/Foundation.h>
#include <stdlib.h>
#include <string.h>

bool nabi_trash_file(const char *path, char **result, char **error) {
    @autoreleasepool {
        NSURL *source = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        NSURL *trashed = nil;
        NSError *failure = nil;
        BOOL ok = [[NSFileManager defaultManager] trashItemAtURL:source resultingItemURL:&trashed error:&failure];
        if (ok && trashed) {
            *result = strdup(trashed.fileSystemRepresentation);
            return true;
        }
        *error = strdup((failure.localizedDescription ?: @"Trash did not return a restore location.").UTF8String);
        return false;
    }
}

bool nabi_restore_file(const char *path, const char *original, char **error) {
    @autoreleasepool {
        NSURL *source = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        NSURL *destination = [NSURL fileURLWithFileSystemRepresentation:original isDirectory:NO relativeToURL:nil];
        NSError *failure = nil;
        // NSFileManager move fails when the destination exists; never replace it.
        BOOL ok = [[NSFileManager defaultManager] moveItemAtURL:source toURL:destination error:&failure];
        if (!ok) *error = strdup((failure.localizedDescription ?: @"Could not restore the file.").UTF8String);
        return ok;
    }
}

void nabi_trash_free(char *value) { free(value); }
