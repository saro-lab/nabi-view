// Bounded PhotoKit metadata access shared by importing and resume validation.
// Owns one source snapshot; never owns originals, pixels or OS permissions.
#import <Foundation/Foundation.h>
#import <Photos/Photos.h>
#include <stdint.h>
#include <stdbool.h>

NSArray<NSDictionary *> *nabi_photo_entries(NSString *identifier);
// Keep the lazy PhotoKit result until its one selected page has been published.
// Resource enumeration is explicit and never part of fetching the list itself.
PHFetchResult<PHAsset *> *nabi_photo_assets(NSString *identifier);
NSDictionary *nabi_photo_entry(PHAsset *asset);
// Only candidates inside the first 1,000 native results may be seeded. An
// explicit name must match the current resource exactly; nil requests the first
// usable resource. current() is checked between PhotoKit calls, not held in a lock.
NSDictionary *nabi_photo_seed_entry(PHFetchResult<PHAsset *> *assets, NSString *name,
                                    NSUInteger *index, bool (^current)(void));
uint64_t nabi_photo_snapshot_revision(void);
void nabi_photo_snapshot_changed(void);
void nabi_photo_snapshot_remember(NSString *identifier, NSArray<NSDictionary *> *entries,
                                   uint64_t revision, bool complete);
// Asynchronously reports (request, same) through the Rust validation callback.
void nabi_photos_validate(const char *source, uint64_t request);
