// Рисует иконку KeepAwake.app (чашка с паром) и собирает assets/AppIcon.icns и assets/icon.png.
// Запуск из корня репозитория: swift scripts/make-icon.swift
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xff) / 255,
        green: CGFloat((hex >> 8) & 0xff) / 255,
        blue: CGFloat(hex & 0xff) / 255,
        alpha: alpha
    )
}

/// Рисует в координатах 1024×1024 (ось y вверх).
func draw(_ ctx: CGContext) {
    // Подложка по сетке иконок macOS: 824×824 с отступом 100.
    let tile = CGPath(
        roundedRect: CGRect(x: 100, y: 100, width: 824, height: 824),
        cornerWidth: 185, cornerHeight: 185, transform: nil
    )
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: color(0x000000, 0.35))
    ctx.addPath(tile)
    ctx.setFillColor(color(0xE8613A))
    ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.addPath(tile)
    ctx.clip()
    let gradient = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB),
        colors: [color(0xFFB65C), color(0xE8603A)] as CFArray,
        locations: [0, 1]
    )!
    ctx.drawLinearGradient(gradient, start: CGPoint(x: 512, y: 924), end: CGPoint(x: 512, y: 100), options: [])
    ctx.restoreGState()

    let white = color(0xFFFFFF)

    // Блюдце.
    ctx.addPath(CGPath(
        roundedRect: CGRect(x: 250, y: 290, width: 524, height: 44),
        cornerWidth: 22, cornerHeight: 22, transform: nil
    ))
    ctx.setFillColor(white)
    ctx.fillPath()

    // Ручка: кольцо справа, левую половину закроет сама чашка.
    ctx.setStrokeColor(white)
    ctx.setLineWidth(44)
    ctx.addArc(center: CGPoint(x: 668, y: 530), radius: 74, startAngle: 0, endAngle: 2 * .pi, clockwise: false)
    ctx.strokePath()

    // Чашка: прямой верх, скруглённое дно.
    let cup = CGMutablePath()
    cup.move(to: CGPoint(x: 318, y: 650))
    cup.addLine(to: CGPoint(x: 666, y: 650))
    cup.addLine(to: CGPoint(x: 666, y: 476))
    cup.addArc(center: CGPoint(x: 552, y: 476), radius: 114, startAngle: 0, endAngle: -.pi / 2, clockwise: true)
    cup.addLine(to: CGPoint(x: 432, y: 362))
    cup.addArc(center: CGPoint(x: 432, y: 476), radius: 114, startAngle: -.pi / 2, endAngle: .pi, clockwise: true)
    cup.closeSubpath()
    ctx.addPath(cup)
    ctx.fillPath()

    // Пар.
    ctx.setStrokeColor(color(0xFFFFFF, 0.9))
    ctx.setLineWidth(30)
    ctx.setLineCap(.round)
    for x in [404.0, 492.0, 580.0] {
        ctx.move(to: CGPoint(x: x, y: 700))
        ctx.addCurve(to: CGPoint(x: x, y: 830), control1: CGPoint(x: x - 44, y: 745), control2: CGPoint(x: x + 44, y: 785))
        ctx.strokePath()
    }
}

func png(size: Int, to url: URL) {
    let ctx = CGContext(
        data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    ctx.scaleBy(x: CGFloat(size) / 1024, y: CGFloat(size) / 1024)
    draw(ctx)
    let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, ctx.makeImage()!, nil)
    precondition(CGImageDestinationFinalize(dest), "не записать \(url.path)")
}

let fm = FileManager.default
let iconset = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("AppIcon.iconset")
try? fm.removeItem(at: iconset)
try fm.createDirectory(at: iconset, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    png(size: base, to: iconset.appendingPathComponent("icon_\(base)x\(base).png"))
    png(size: base * 2, to: iconset.appendingPathComponent("icon_\(base)x\(base)@2x.png"))
}

try fm.createDirectory(atPath: "assets", withIntermediateDirectories: true)
png(size: 256, to: URL(fileURLWithPath: "assets/icon.png"))

let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", iconset.path, "-o", "assets/AppIcon.icns"]
try iconutil.run()
iconutil.waitUntilExit()
precondition(iconutil.terminationStatus == 0, "iconutil завершился с ошибкой")
print("assets/AppIcon.icns, assets/icon.png")
