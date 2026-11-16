import AppKit

// Lays the rendered ribbon (scripts/blender/icon.py) on the app icon's tile.
//
//     swift assets/branding/tile.swift ribbon.png out.png light|dark
//
// Apple's grid for a macOS icon: a 1024 canvas, an 824 tile in the middle, its corners
// rounded, transparent outside. Light is the white tile the icon has always had; dark is
// the cell the app opens onto.
let args = CommandLine.arguments
guard args.count == 4,
      let source = NSImage(contentsOfFile: args[1]),
      let ribbon = source.cgImage(forProposedRect: nil, context: nil, hints: nil)
else { fatalError("Usage: tile.swift ribbon.png out.png light|dark") }
let dark = args[3] == "dark"

let size: CGFloat = 1024
let space = CGColorSpace(name: CGColorSpace.sRGB)!
let context = CGContext(data: nil, width: Int(size), height: Int(size),
    bitsPerComponent: 8, bytesPerRow: 0, space: space,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
context.interpolationQuality = .high

let lo: CGFloat = 100, hi: CGFloat = 924, r: CGFloat = 185
let k: CGFloat = r * 0.65
let tile = CGMutablePath()
tile.move(to: CGPoint(x: lo + r, y: lo))
tile.addLine(to: CGPoint(x: hi - r, y: lo))
tile.addCurve(to: CGPoint(x: hi, y: lo + r), control1: CGPoint(x: hi - r + k, y: lo), control2: CGPoint(x: hi, y: lo + r - k))
tile.addLine(to: CGPoint(x: hi, y: hi - r))
tile.addCurve(to: CGPoint(x: hi - r, y: hi), control1: CGPoint(x: hi, y: hi - r + k), control2: CGPoint(x: hi - r + k, y: hi))
tile.addLine(to: CGPoint(x: lo + r, y: hi))
tile.addCurve(to: CGPoint(x: lo, y: hi - r), control1: CGPoint(x: lo + r - k, y: hi), control2: CGPoint(x: lo, y: hi - r + k))
tile.addLine(to: CGPoint(x: lo, y: lo + r))
tile.addCurve(to: CGPoint(x: lo + r, y: lo), control1: CGPoint(x: lo, y: lo + r - k), control2: CGPoint(x: lo + r - k, y: lo))
tile.closeSubpath()

// The tile's own soft shadow, as macOS icons have, under it.
context.saveGState()
context.setShadow(offset: CGSize(width: 0, height: -10), blur: 28,
    color: CGColor(srgbRed: 0, green: 0, blue: 0, alpha: 0.35))
context.addPath(tile)
context.setFillColor(CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 1))
context.fillPath()
context.restoreGState()

context.saveGState()
context.addPath(tile)
context.clip()
let colours: [CGColor] = dark
    ? [CGColor(srgbRed: 0.10, green: 0.17, blue: 0.27, alpha: 1), CGColor(srgbRed: 0.02, green: 0.05, blue: 0.10, alpha: 1)]
    : [CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 1), CGColor(srgbRed: 0.93, green: 0.94, blue: 0.96, alpha: 1)]
let gradient = CGGradient(colorsSpace: space, colors: colours as CFArray, locations: [0, 1])!
// Lit from above: the glow of the cell's haze in the dark tile, daylight in the light one.
context.drawRadialGradient(gradient, startCenter: CGPoint(x: size / 2, y: size * 0.62), startRadius: 0,
    endCenter: CGPoint(x: size / 2, y: size * 0.5), endRadius: size * 0.62, options: [.drawsAfterEndLocation])
context.draw(ribbon, in: CGRect(x: 0, y: 0, width: size, height: size))
context.restoreGState()

let bitmap = NSBitmapImageRep(cgImage: context.makeImage()!)
try bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[2]))
print("Saved \(args[2])")
