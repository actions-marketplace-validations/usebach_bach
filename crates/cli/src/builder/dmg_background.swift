// Draws the background of a macOS setup disk image: a calm warm page with the title and one line
// under it. Run as `swift dmg_background.swift OUT.png TITLE SUBTITLE`; the picture is 560 by 360
// points at twice the resolution, so it stays sharp on a Retina screen. Needs nothing but the
// Swift that comes with Xcode's command line tools.
import AppKit

let arguments = CommandLine.arguments
guard arguments.count >= 4 else { exit(2) }
let output = arguments[1], title = arguments[2], subtitle = arguments[3]
let width = 560, height = 360, scale = 2

guard let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: width * scale, pixelsHigh: height * scale,
    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
    colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { exit(1) }
rep.size = NSSize(width: width, height: height)

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

let page = NSRect(x: 0, y: 0, width: width, height: height)
let top = NSColor(srgbRed: 0.949, green: 0.945, blue: 0.929, alpha: 1)
let bottom = NSColor(srgbRed: 0.929, green: 0.925, blue: 0.906, alpha: 1)
NSGradient(starting: top, ending: bottom)?.draw(in: page, angle: -90)

func serif(_ size: CGFloat, _ weight: NSFont.Weight) -> NSFont {
    let base = NSFont.systemFont(ofSize: size, weight: weight)
    if let design = base.fontDescriptor.withDesign(.serif), let font = NSFont(descriptor: design, size: size) { return font }
    return base
}

func centered(_ text: String, font: NSFont, color: NSColor, baseline: CGFloat) {
    let style = NSMutableParagraphStyle()
    style.alignment = .center
    style.lineBreakMode = .byTruncatingTail
    let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color, .paragraphStyle: style]
    let box = NSRect(x: 24, y: baseline, width: CGFloat(width) - 48, height: font.pointSize * 1.5)
    NSAttributedString(string: text, attributes: attributes).draw(in: box)
}

centered(title, font: serif(34, .semibold), color: NSColor(srgbRed: 0.09, green: 0.09, blue: 0.08, alpha: 1), baseline: CGFloat(height) - 78)
centered(subtitle, font: NSFont.systemFont(ofSize: 16, weight: .regular), color: NSColor(srgbRed: 0.42, green: 0.41, blue: 0.39, alpha: 1), baseline: CGFloat(height) - 108)

NSGraphicsContext.restoreGraphicsState()
guard let data = rep.representation(using: .png, properties: [:]) else { exit(1) }
do { try data.write(to: URL(fileURLWithPath: output)) } catch { exit(1) }
