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
      input.width == 1024, input.height == 1024,
      let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)
      else { fail("Pass the shared 1024px icon, AppIcon.png and optional AppIconTinted.png") }

// The shared master already sets size and alignment for every platform.
let size = input.width
let placement = CGRect(x: 0, y: 0, width: size, height: size)

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
light.draw(input, in: placement)
try writePNG(light, to: CommandLine.arguments[2])

if CommandLine.arguments.count == 4 {
    guard let graySpace = CGColorSpace(name: CGColorSpace.genericGrayGamma2_2),
          let tinted = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
            bytesPerRow: size, space: graySpace, bitmapInfo: CGImageAlphaInfo.none.rawValue)
          else { fail("Cannot create tinted icon") }
    tinted.setFillColor(CGColor(gray: 0, alpha: 1))
    tinted.fill(CGRect(x: 0, y: 0, width: size, height: size))
    tinted.interpolationQuality = .high
    tinted.clip(to: placement, mask: input)
    tinted.setFillColor(CGColor(gray: 1, alpha: 1))
    tinted.fill(placement)
    try writePNG(tinted, to: CommandLine.arguments[3])
}
