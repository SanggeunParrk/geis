import AppKit

// Preserve the approved artwork; make the tile's perimeter the actual alpha edge.
// Coordinates are measured on the 1254px source. Inset removes the baked gray rim.
let args = CommandLine.arguments
guard args.count == 3,
      let source = NSImage(contentsOfFile: args[1]),
      let image = source.cgImage(forProposedRect: nil, context: nil, hints: nil)
else { fatalError("Usage: prepare-icon.swift source.png output.png") }
let size = 1254
let scale = 2
let space = CGColorSpace(name: CGColorSpace.sRGB)!
let context = CGContext(data: nil, width: size * scale, height: size * scale,
    bitsPerComponent: 8, bytesPerRow: 0, space: space,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
context.scaleBy(x: CGFloat(scale), y: CGFloat(scale))
context.setAllowsAntialiasing(true)
context.setShouldAntialias(true)
let lo: CGFloat = 73, hi: CGFloat = 1181, r: CGFloat = 235
let k: CGFloat = r * 0.65
let path = CGMutablePath()
path.move(to: CGPoint(x: lo + r, y: lo))
path.addLine(to: CGPoint(x: hi - r, y: lo))
path.addCurve(to: CGPoint(x: hi, y: lo + r),
    control1: CGPoint(x: hi - r + k, y: lo), control2: CGPoint(x: hi, y: lo + r - k))
path.addLine(to: CGPoint(x: hi, y: hi - r))
path.addCurve(to: CGPoint(x: hi - r, y: hi),
    control1: CGPoint(x: hi, y: hi - r + k), control2: CGPoint(x: hi - r + k, y: hi))
path.addLine(to: CGPoint(x: lo + r, y: hi))
path.addCurve(to: CGPoint(x: lo, y: hi - r),
    control1: CGPoint(x: lo + r - k, y: hi), control2: CGPoint(x: lo, y: hi - r + k))
path.addLine(to: CGPoint(x: lo, y: lo + r))
path.addCurve(to: CGPoint(x: lo + r, y: lo),
    control1: CGPoint(x: lo, y: lo + r - k), control2: CGPoint(x: lo + r - k, y: lo))
path.closeSubpath()
context.addPath(path)
context.clip()
context.interpolationQuality = .high
context.draw(image, in: CGRect(x: 0, y: 0, width: size, height: size))
let output = CGContext(data: nil, width: 1024, height: 1024,
    bitsPerComponent: 8, bytesPerRow: 0, space: space,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
output.interpolationQuality = .high
output.draw(context.makeImage()!, in: CGRect(x: 0, y: 0, width: 1024, height: 1024))
let bitmap = NSBitmapImageRep(cgImage: output.makeImage()!)
try bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[2]))
print("Saved 1024x1024 RGBA icon to \(args[2])")
