#import <IOKit/pwr_mgt/IOPMLib.h>

int32_t nabi_display_sleep_acquire(uint32_t *assertion) {
    return IOPMAssertionCreateWithName(
        kIOPMAssertionTypePreventUserIdleDisplaySleep,
        kIOPMAssertionLevelOn,
        CFSTR("NABI VIEW: viewing images"),
        assertion);
}

void nabi_display_sleep_release(uint32_t assertion) {
    IOPMAssertionRelease(assertion);
}
