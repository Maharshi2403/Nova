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
    mark.move(to: NSPoint(x: n * 0.25, y: n * 0.25))
    mark.line(to: NSPoint(x: n * 0.25, y: n * 0.75))
    mark.line(to: NSPoint(x: n * 0.70, y: n * 0.25))
    mark.line(to: NSPoint(x: n * 0.70, y: n * 0.75))
    mark.lineWidth = n * 0.09
    mark.lineCapStyle = .round
    mark.lineJoinStyle = .round
    NSColor.white.setStroke()
    mark.stroke()
    NSColor.white.setFill()
    NSBezierPath(ovalIn: NSRect(x: n * 0.80, y: n * 0.78, width: n * 0.07, height: n * 0.07)).fill()
    NSGraphicsContext.restoreGraphicsState()
    let suffix = scale == 2 ? "@2x" : ""
    try bitmap.representation(using: .png, properties: [:])!.write(
      to: folder.appendingPathComponent("icon_\(size)x\(size)\(suffix).png"))
  }
}
