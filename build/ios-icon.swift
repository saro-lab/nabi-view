// Render centered iPhone/iPad icon variants from the shared app artwork.
// Usage: swift build/ios-icon.swift SOURCE.png AppIcon.png [AppIconTinted.png]
import AppKit

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

guard (3...4).contains(CommandLine.arguments.count),
      let source = NSImage(contentsOfFile: CommandLine.arguments[1]),
      let input = source.cgImage(forProposedRect: nil, context: nil, hints: nil),
      let colorSpace = CGColorSpace(name: CGColorSpace.sRGB),
      let sourceContext = CGContext(data: nil, width: input.width, height: input.height,
        bitsPerComponent: 8, bytesPerRow: input.width * 4, space: colorSpace,
        bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue)
      else { fail("Usage: swift build/ios-icon.swift SOURCE.png AppIcon.png [AppIconTinted.png]") }

sourceContext.draw(input, in: CGRect(x: 0, y: 0, width: input.width, height: input.height))
guard let data = sourceContext.data?.assumingMemoryBound(to: UInt8.self),
      let decoded = sourceContext.makeImage() else { fail("Cannot decode source icon") }
var minX = input.width, minY = input.height, maxX = -1, maxY = -1
for y in 0..<input.height {
    for x in 0..<input.width where data[y * sourceContext.bytesPerRow + x * 4 + 3] != 0 {
        minX = min(minX, x); minY = min(minY, y)
        maxX = max(maxX, x); maxY = max(maxY, y)
    }
}
guard maxX >= minX, maxY >= minY else { fail("Source icon is fully transparent") }
let bounds = CGRect(x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1)
guard let artwork = decoded.cropping(to: bounds) else { fail("Cannot crop source icon") }

// Center the visible artwork, not the source canvas. Leave room for the system mask.
let size = 1024
let scale = 820 / max(bounds.width, bounds.height)
let width = bounds.width * scale, height = bounds.height * scale
let placement = CGRect(x: (CGFloat(size) - width) / 2, y: (CGFloat(size) - height) / 2,
    width: width, height: height)

func writePNG(_ context: CGContext, to path: String) throws {
    guard let output = context.makeImage(),
          let png = NSBitmapImageRep(cgImage: output).representation(using: .png, properties: [:])
          else { fail("Cannot encode icon: \(path)") }
    try png.write(to: URL(fileURLWithPath: path))
}

guard let light = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
    bytesPerRow: size * 4, space: colorSpace, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
    else { fail("Cannot create light icon") }
light.setFillColor(CGColor(gray: 1, alpha: 1))
light.fill(CGRect(x: 0, y: 0, width: size, height: size))
light.interpolationQuality = .high
light.draw(artwork, in: placement)
try writePNG(light, to: CommandLine.arguments[2])

if CommandLine.arguments.count == 4 {
    guard let graySpace = CGColorSpace(name: CGColorSpace.genericGrayGamma2_2),
          let tinted = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
            bytesPerRow: size, space: graySpace, bitmapInfo: CGImageAlphaInfo.none.rawValue)
          else { fail("Cannot create tinted icon") }
    tinted.setFillColor(CGColor(gray: 0, alpha: 1))
    tinted.fill(CGRect(x: 0, y: 0, width: size, height: size))
    tinted.interpolationQuality = .high
    tinted.clip(to: placement, mask: artwork)
    tinted.setFillColor(CGColor(gray: 1, alpha: 1))
    tinted.fill(placement)
    try writePNG(tinted, to: CommandLine.arguments[3])
}
print("Source alpha bounds: \(bounds); centered icon artwork: \(placement)")
