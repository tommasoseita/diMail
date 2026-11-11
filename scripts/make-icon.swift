// Prepara l'icona macOS di Dimail dal logo: trova il quadrato arrotondato dentro lo sfondo nero,
// lo ritaglia e lo mette nella griglia di Apple (tela 1024, corpo 824 con 100 px di margine,
// angoli raggio 185,4), con gli angoli trasparenti.
// Uso: swift scripts/make-icon.swift design/logo.webp design/app-icon-1024.png /tmp/anteprima.png
//      poi: npx tauri icon design/app-icon-1024.png
import AppKit
import CoreGraphics

let args = CommandLine.arguments
guard args.count == 4,
      let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, nil),
      let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
    fatalError("immagine non leggibile")
}

// Pixel RGBA per misurare.
let w = img.width, h = img.height
var px = [UInt8](repeating: 0, count: w * h * 4)
let cs = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(data: &px, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: cs,
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
// Nel buffer la riga 0 è in alto.
func lum(_ x: Int, _ y: Int) -> Int {
    let i = (y * w + x) * 4
    return max(Int(px[i]), Int(px[i + 1]), Int(px[i + 2]))
}
let dark = 24
var minX = w, minY = h, maxX = 0, maxY = 0
for y in 0..<h { for x in 0..<w where lum(x, y) > dark {
    minX = min(minX, x); maxX = max(maxX, x); minY = min(minY, y); maxY = max(maxY, y)
} }
// Raggio degli angoli: lungo la diagonale dall'angolo in alto a sinistra, il primo pixel chiaro
// è a r·(1 − 1/√2) dal bordo.
var t = 0
while minX + t < maxX && minY + t < maxY && lum(minX + t, minY + t) <= dark { t += 1 }
let radius = Double(t) / (1 - 1 / 2.0.squareRoot())
print("riquadro: x \(minX)…\(maxX), y \(minY)…\(maxY) (\(maxX - minX + 1)×\(maxY - minY + 1)), raggio angoli ≈ \(Int(radius)) px")

// Ritaglio quadrato centrato sul riquadro trovato.
let side = max(maxX - minX, maxY - minY) + 1
let cx = (minX + maxX + 1) / 2, cy = (minY + maxY + 1) / 2
let crop = img.cropping(to: CGRect(x: cx - side / 2, y: cy - side / 2, width: side, height: side))!

func render(canvas: Int, body: Double, out: String) {
    let c = CGContext(data: nil, width: canvas, height: canvas, bitsPerComponent: 8, bytesPerRow: 0, space: cs,
                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    c.clear(CGRect(x: 0, y: 0, width: canvas, height: canvas))
    let margin = (Double(canvas) - body) / 2
    let rect = CGRect(x: margin, y: margin, width: body, height: body)
    // Forma della griglia di Apple. Il logo ha angoli più tondi: lo si ingrandisce del 4%, così i suoi
    // angoli (neri attorno) cadono fuori dalla forma e il bordo resta pulito.
    let r = 185.4 / 824 * body
    c.addPath(CGPath(roundedRect: rect, cornerWidth: r, cornerHeight: r, transform: nil))
    c.clip()
    c.interpolationQuality = .high
    c.draw(crop, in: rect.insetBy(dx: -body * 0.02, dy: -body * 0.02))
    // Controllo: appena dentro il bordo arrotondato non deve esserci nero.
    let outImg = c.makeImage()!
    var buf = [UInt8](repeating: 0, count: canvas * canvas * 4)
    let vc = CGContext(data: &buf, width: canvas, height: canvas, bitsPerComponent: 8, bytesPerRow: canvas * 4, space: cs,
                       bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    vc.draw(outImg, in: CGRect(x: 0, y: 0, width: canvas, height: canvas))
    var darkest = 255
    for corner in 0..<4 {
        let (sx, sy): (Double, Double) = [(1, 1), (-1, 1), (1, -1), (-1, -1)][corner]
        let ccx = Double(canvas) / 2 - sx * (body / 2 - r), ccy = Double(canvas) / 2 - sy * (body / 2 - r)
        for step in 0...90 {
            let a = Double(step) * .pi / 180
            let x = Int(ccx - sx * (r - 4) * cos(a)), y = Int(ccy - sy * (r - 4) * sin(a))
            let i = (y * canvas + x) * 4
            darkest = min(darkest, max(Int(buf[i]), Int(buf[i + 1]), Int(buf[i + 2])))
        }
    }
    print("pixel più scuro lungo il bordo degli angoli: \(darkest) (nero = 0)")
    let rep = NSBitmapImageRep(cgImage: outImg)
    try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
    print("scritto \(out) (\(canvas)×\(canvas), raggio \(Int(r)))")
}

render(canvas: 1024, body: 824, out: args[2])
// Anteprima su sfondo chiaro, per controllare bordi e trasparenza a occhio.
render(canvas: 1024, body: 824, out: args[3])
