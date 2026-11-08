// Build-time artwork only. Finder supplies the two draggable file icons.
#import <Cocoa/Cocoa.h>

static NSColor *color(CGFloat r, CGFloat g, CGFloat b) {
    return [NSColor colorWithSRGBRed:r green:g blue:b alpha:1];
}

static void text(NSString *value, CGFloat y, CGFloat size, NSFontWeight weight, NSColor *ink) {
    NSMutableParagraphStyle *style = [NSMutableParagraphStyle new];
    style.alignment = NSTextAlignmentCenter;
    [value drawInRect:NSMakeRect(30, y, 600, size + 12) withAttributes:@{
        NSFontAttributeName: [NSFont systemFontOfSize:size weight:weight],
        NSForegroundColorAttributeName: ink,
        NSParagraphStyleAttributeName: style
    }];
}

static NSBitmapImageRep *render(NSInteger scale, NSString *iconPath, BOOL preview) {
    NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc]
        initWithBitmapDataPlanes:NULL pixelsWide:660 * scale pixelsHigh:440 * scale
        bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES isPlanar:NO
        colorSpaceName:NSCalibratedRGBColorSpace bytesPerRow:0 bitsPerPixel:0];
    bitmap.size = NSMakeSize(660, 440);
    [NSGraphicsContext saveGraphicsState];
    NSGraphicsContext *bitmapContext = [NSGraphicsContext graphicsContextWithBitmapImageRep:bitmap];
    [NSGraphicsContext setCurrentContext:[NSGraphicsContext graphicsContextWithCGContext:bitmapContext.CGContext flipped:YES]];
    NSAffineTransform *transform = [NSAffineTransform transform];
    [transform translateXBy:0 yBy:440];
    [transform scaleXBy:1 yBy:-1];
    [transform concat];

    [color(0.97, 0.98, 0.97) setFill];
    NSRectFill(NSMakeRect(0, 0, 660, 440));
    NSBezierPath *hill = [NSBezierPath bezierPath];
    [hill moveToPoint:NSMakePoint(0, 390)];
    [hill curveToPoint:NSMakePoint(660, 350) controlPoint1:NSMakePoint(250, 285) controlPoint2:NSMakePoint(470, 440)];
    [hill lineToPoint:NSMakePoint(660, 440)];
    [hill lineToPoint:NSMakePoint(0, 440)];
    [hill closePath];
    [color(0.92, 0.95, 0.92) setFill];
    [hill fill];

    text(@"NABI VIEW", 53, 30, NSFontWeightSemibold, color(0.16, 0.24, 0.21));
    text(@"Drag to Applications to install", 99, 15, NSFontWeightRegular, color(0.39, 0.46, 0.42));
    for (NSNumber *x in @[@180, @480]) {
        NSRect pad = NSMakeRect(x.doubleValue - 78, 158, 156, 160);
        [NSColor.whiteColor setFill];
        [[NSBezierPath bezierPathWithRoundedRect:pad xRadius:28 yRadius:28] fill];
    }
    NSBezierPath *arrow = [NSBezierPath bezierPath];
    [arrow moveToPoint:NSMakePoint(299, 230)];
    [arrow lineToPoint:NSMakePoint(361, 230)];
    [arrow moveToPoint:NSMakePoint(348, 217)];
    [arrow lineToPoint:NSMakePoint(361, 230)];
    [arrow lineToPoint:NSMakePoint(348, 243)];
    arrow.lineWidth = 3;
    arrow.lineCapStyle = NSLineCapStyleRound;
    arrow.lineJoinStyle = NSLineJoinStyleRound;
    [color(0.37, 0.55, 0.44) setStroke];
    [arrow stroke];
    text(@"나비 아이콘을 오른쪽 응용 프로그램 폴더로 드래그하세요", 366, 13, NSFontWeightRegular, color(0.34, 0.43, 0.37));

    if (preview) {
        NSArray<NSImage *> *icons = @[
            [[NSImage alloc] initWithContentsOfFile:iconPath],
            [[NSImage alloc] initWithContentsOfFile:@"/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/ApplicationsFolderIcon.icns"]
                ?: [NSImage imageNamed:NSImageNameFolder]
        ];
        NSArray<NSString *> *names = @[@"NABI VIEW", @"Applications"];
        for (NSUInteger i = 0; i < 2; ++i) {
            CGFloat x = i == 0 ? 180 : 480;
            [icons[i] drawInRect:NSMakeRect(x - 56, 174, 112, 112) fromRect:NSZeroRect
                operation:NSCompositingOperationSourceOver fraction:1 respectFlipped:YES hints:nil];
            NSDictionary *attributes = @{
                NSFontAttributeName: [NSFont systemFontOfSize:13],
                NSForegroundColorAttributeName: color(0.16, 0.24, 0.21)
            };
            NSSize size = [names[i] sizeWithAttributes:attributes];
            [names[i] drawAtPoint:NSMakePoint(x - size.width / 2, 292) withAttributes:attributes];
        }
    }
    [NSGraphicsContext restoreGraphicsState];
    return bitmap;
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 4) {
            fprintf(stderr, "Usage: dmg-artwork background.tiff preview.png app-icon.png\n");
            return 2;
        }
        NSString *icon = [NSString stringWithUTF8String:argv[3]];
        if (![[NSFileManager defaultManager] fileExistsAtPath:icon]) return 1;
        NSArray *reps = @[render(1, icon, NO), render(2, icon, NO)];
        NSData *tiff = [NSBitmapImageRep TIFFRepresentationOfImageRepsInArray:reps
            usingCompression:NSTIFFCompressionLZW factor:1];
        NSData *preview = [render(2, icon, YES) representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
        NSError *error = nil;
        if (![tiff writeToFile:[NSString stringWithUTF8String:argv[1]] options:0 error:&error] ||
            ![preview writeToFile:[NSString stringWithUTF8String:argv[2]] options:0 error:&error]) {
            fprintf(stderr, "%s\n", error.localizedDescription.UTF8String);
            return 1;
        }
    }
    return 0;
}
