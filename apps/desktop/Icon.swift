import AppKit

// Vector-rendered application mark; no external assets or image service.
let folder = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
for size in [16, 32, 128, 256, 512] {
  for scale in [1, 2] {
    let pixels = size * scale
    let bitmap = NSBitmapImageRep(
      bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
      bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
      colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)!
    let n = CGFloat(pixels)
    NSColor(calibratedRed: 0.9, green: 0.32, blue: 0.17, alpha: 1).setFill()
    NSBezierPath(
      roundedRect: NSRect(x: 0, y: 0, width: n, height: n), xRadius: n * 0.22, yRadius: n * 0.22
    ).fill()
    let mark = NSBezierPath()
    mark.move(to: NSPoint(x: n * 0.5, y: n * 0.82))
    mark.curve(
      to: NSPoint(x: n * 0.82, y: n * 0.5), controlPoint1: NSPoint(x: n * 0.55, y: n * 0.57),
      controlPoint2: NSPoint(x: n * 0.57, y: n * 0.55))
    mark.curve(
      to: NSPoint(x: n * 0.5, y: n * 0.18), controlPoint1: NSPoint(x: n * 0.57, y: n * 0.45),
      controlPoint2: NSPoint(x: n * 0.55, y: n * 0.43))
    mark.curve(
      to: NSPoint(x: n * 0.18, y: n * 0.5), controlPoint1: NSPoint(x: n * 0.45, y: n * 0.43),
      controlPoint2: NSPoint(x: n * 0.43, y: n * 0.45))
    mark.curve(
      to: NSPoint(x: n * 0.5, y: n * 0.82), controlPoint1: NSPoint(x: n * 0.43, y: n * 0.55),
      controlPoint2: NSPoint(x: n * 0.45, y: n * 0.57))
    NSColor.white.setFill()
    mark.fill()
    NSGraphicsContext.restoreGraphicsState()
    let suffix = scale == 2 ? "@2x" : ""
    try bitmap.representation(using: .png, properties: [:])!.write(
      to: folder.appendingPathComponent("icon_\(size)x\(size)\(suffix).png"))
  }
}
