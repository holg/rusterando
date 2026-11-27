import SwiftUI

/// The Tab5's own palette (crates/tab5-lab/src/console.rs), so the phone
/// looks like the screen it mirrors.
enum Theme {
    static let background = Color(hex: 0x121418)
    static let barBackground = Color(hex: 0x1F4E86)
    static let barText = Color.white
    static let panel = Color(hex: 0x1C1F25)
    static let font = Font.system(size: 13, design: .monospaced)
}

extension Tone {
    var color: Color {
        switch self {
        case .normal: return Color(hex: 0xD0D0D0)
        case .dim: return Color(hex: 0x808890)
        case .echo: return Color(hex: 0xFFFFFF)
        case .ok: return Color(hex: 0x6FDC6F)
        case .received: return Color(hex: 0x5FD7FF)
        case .sent: return Color(hex: 0xFFD75F)
        case .warning: return Color(hex: 0xFFA040)
        case .error: return Color(hex: 0xFF6060)
        }
    }
}

extension BLEManager.Phase {
    var label: String {
        switch self {
        case .unavailable(let why): return why
        case .idle: return "idle"
        case .scanning: return "scanning"
        case .connecting: return "connecting"
        case .waitingForKey: return "waiting for key"
        case .needsKey(let why): return why.lowercased()
        case .connected: return "connected"
        case .disconnected: return "disconnected"
        }
    }

    var color: Color {
        switch self {
        case .connected: return Tone.ok.color
        case .connecting, .waitingForKey, .scanning: return Tone.sent.color
        case .needsKey, .disconnected, .unavailable: return Tone.error.color
        case .idle: return Tone.dim.color
        }
    }
}

extension Color {
    init(hex: UInt32) {
        self.init(
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}
