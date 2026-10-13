#import <Foundation/Foundation.h>
#import <Sparkle/Sparkle.h>
#include <stdbool.h>

static SPUStandardUpdaterController *OpenWalkieTalkieUpdaterController;

bool owt_sparkle_is_enabled(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenWalkieTalkie updater configuration must be read on the main thread");
        return false;
    }
    return [NSBundle.mainBundle.infoDictionary[@"OpenWalkieTalkieAutomaticUpdates"] boolValue];
}

bool owt_sparkle_start(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenWalkieTalkie updater must start on the main thread");
        return false;
    }

    NSDictionary *info = NSBundle.mainBundle.infoDictionary;
    if (!owt_sparkle_is_enabled()) {
        return false;
    }

    NSString *feedURL = info[@"SUFeedURL"];
    NSString *publicKey = info[@"SUPublicEDKey"];
    if (![feedURL isKindOfClass:NSString.class] || feedURL.length == 0 ||
        ![publicKey isKindOfClass:NSString.class] || publicKey.length == 0 ||
        ![info[@"SURequireSignedFeed"] boolValue] ||
        ![info[@"SUVerifyUpdateBeforeExtraction"] boolValue]) {
        NSLog(@"OpenWalkieTalkie updater is disabled because its signed feed is not configured");
        return false;
    }

    if (OpenWalkieTalkieUpdaterController == nil) {
        OpenWalkieTalkieUpdaterController = [[SPUStandardUpdaterController alloc]
            initWithStartingUpdater:YES
                 updaterDelegate:nil
              userDriverDelegate:nil];
    }
    if (OpenWalkieTalkieUpdaterController.updater.automaticallyChecksForUpdates) {
        [OpenWalkieTalkieUpdaterController.updater checkForUpdatesInBackground];
    }
    return OpenWalkieTalkieUpdaterController != nil;
}

bool owt_sparkle_can_check_for_updates(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenWalkieTalkie updater checks must run on the main thread");
        return false;
    }
    return OpenWalkieTalkieUpdaterController != nil &&
           OpenWalkieTalkieUpdaterController.updater.canCheckForUpdates;
}

bool owt_sparkle_session_in_progress(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenWalkieTalkie updater state must be read on the main thread");
        return false;
    }
    return OpenWalkieTalkieUpdaterController != nil &&
           OpenWalkieTalkieUpdaterController.updater.sessionInProgress;
}

bool owt_sparkle_check_for_updates(void) {
    if (![NSThread isMainThread]) {
        NSLog(@"OpenWalkieTalkie updater checks must run on the main thread");
        return false;
    }
    if (OpenWalkieTalkieUpdaterController == nil ||
        !OpenWalkieTalkieUpdaterController.updater.canCheckForUpdates) {
        return false;
    }
    [OpenWalkieTalkieUpdaterController checkForUpdates:nil];
    return true;
}
