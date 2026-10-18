// Minimal AppKit status-item app, used by leakcheck.sh as the floor for what
// Apple's `leaks` reports on *any* menu bar app on this macOS version. It has
// no Rust, no objc2 and no IOKit; whatever it leaks is AppKit's, not ours.
#import <Cocoa/Cocoa.h>

int main(void) {
    @autoreleasepool {
        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyAccessory];
        NSStatusItem *item = [[NSStatusBar systemStatusBar]
            statusItemWithLength:NSVariableStatusItemLength];
        // the app's menu bar icon, loaded the way the app loads it (run from the repository root)
        NSImage *img = [[NSImage alloc]
            initWithData:[NSData dataWithContentsOfFile:@"assets/StatusIcon.tiff"]];
        item.button.image = img;
        item.button.font = [NSFont monospacedDigitSystemFontOfSize:[NSFont menuBarFontOfSize:0].pointSize
                                                            weight:NSFontWeightRegular];
        NSMenu *menu = [[NSMenu alloc] init];
        [menu addItemWithTitle:@"Baseline" action:nil keyEquivalent:@""];
        [menu addItem:[NSMenuItem separatorItem]];
        [menu addItemWithTitle:@"Quit" action:@selector(terminate:) keyEquivalent:@"q"];
        item.menu = menu;
        [NSTimer scheduledTimerWithTimeInterval:1.0 repeats:YES block:^(NSTimer *t) {
            item.button.title = [NSString stringWithFormat:@"%ld", (long)time(NULL) % 100];
        }];
        [app run];
    }
    return 0;
}
