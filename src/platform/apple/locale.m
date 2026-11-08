// API review: https://developer.apple.com/documentation/foundation/locale/preferredlanguages
#import <Foundation/Foundation.h>
#include <string.h>

size_t nabi_locale_languages(unsigned char *buffer, size_t capacity) {
    @autoreleasepool {
        NSData *data = [[[NSLocale preferredLanguages] componentsJoinedByString:@","] dataUsingEncoding:NSUTF8StringEncoding];
        if (!data) return 0;
        size_t size = data.length + 1;
        if (buffer && capacity >= size) {
            memcpy(buffer, data.bytes, data.length);
            buffer[data.length] = 0;
        }
        return size;
    }
}
