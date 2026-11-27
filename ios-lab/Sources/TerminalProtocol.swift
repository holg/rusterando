import CryptoKit
import Foundation

/// The Tab5 lab firmware's phone protocol (crates/tab5-lab/src/remote.rs):
/// UTF-8 text lines ending in "\n", the first character says what. Pure —
/// no Bluetooth in here, so it is tested on its own.
enum TerminalProtocol {
    /// Nordic UART Service, advertised by the Tab5.
    static let serviceUUID = "6E400001-B5A3-F393-E0A9-E50E24DCCA9E"
    /// The phone writes here.
    static let rxUUID = "6E400002-B5A3-F393-E0A9-E50E24DCCA9E"
    /// The Tab5 notifies here.
    static let txUUID = "6E400003-B5A3-F393-E0A9-E50E24DCCA9E"

    /// One line from the Tab5, without its "\n".
    static func parse(_ line: String) -> Incoming {
        // Like the firmware: the kind runs up to the first space, the rest
        // is kept exactly (prompts end in a space, status may start with one).
        let kind: Substring
        let rest: String
        if let space = line.firstIndex(of: " ") {
            kind = line[..<space]
            rest = String(line[line.index(after: space)...])
        } else {
            kind = Substring(line)
            rest = ""
        }

        switch kind {
        case "H":
            // name version nonce — the name taken as the rest, to be lenient.
            let words = rest.split(separator: " ")
            guard words.count >= 3,
                  let nonce = unhex(String(words[words.count - 1])), nonce.count == 16
            else { return .unknown(line) }
            return .hello(
                name: words[..<(words.count - 2)].joined(separator: " "),
                version: String(words[words.count - 2]),
                nonce: nonce
            )
        case "O": return .accepted
        case "E": return .error(rest)
        case "X": return .clear
        case "S": return .status(rest)
        case "P": return .prompt(rest)
        default:
            // "L" plus one tone letter; an unknown letter still shows the text.
            if kind.count == 2, kind.first == "L", let letter = kind.last {
                return .line(Tone(rawValue: letter) ?? .normal, rest)
            }
            return .unknown(line)
        }
    }

    /// A message to the Tab5, "\n" included.
    static func encode(_ message: Outgoing) -> Data {
        let text: String
        switch message {
        case .auth(let hmac): text = "A \(hmac)"
        // A newline inside would end the message early: the Tab5 would
        // take the rest as a second, unknown message.
        // ("\r\n" is one Character in Swift, so it becomes one space.)
        case .command(let line): text = "C " + line.split(whereSeparator: \.isNewline).joined(separator: " ")
        case .key(let key): text = "K \(key.rawValue)"
        }
        return Data((text + "\n").utf8)
    }

    /// The login answer: HMAC-SHA256 keyed with the shared key's UTF-8
    /// bytes over the nonce's raw bytes, lowercase hex. The key itself
    /// never goes over the air.
    static func authAnswer(key: String, nonce: Data) -> String {
        let mac = HMAC<SHA256>.authenticationCode(for: nonce, using: SymmetricKey(data: Data(key.utf8)))
        return hex(Data(mac))
    }

    static func hex(_ data: Data) -> String {
        data.map { String(format: "%02x", $0) }.joined()
    }

    static func unhex(_ text: String) -> Data? {
        let digits = Array(text.utf8)
        guard digits.count % 2 == 0 else { return nil }
        var bytes = Data(capacity: digits.count / 2)
        for i in stride(from: 0, to: digits.count, by: 2) {
            guard let byte = UInt8(String(decoding: digits[i ..< i + 2], as: UTF8.self), radix: 16)
            else { return nil }
            bytes.append(byte)
        }
        return bytes
    }
}

/// How a screen line is coloured; the letter after "L".
enum Tone: Character, CaseIterable {
    case normal = "n", dim = "d", echo = "e", ok = "o"
    case received = "r", sent = "t", warning = "w", error = "x"
}

/// Tab5 → phone.
enum Incoming: Equatable {
    /// On each connection; the nonce is the login challenge (16 bytes).
    case hello(name: String, version: String, nonce: Data)
    /// The key was right.
    case accepted
    /// Refused or failed: "wrong key", "log in first", …
    case error(String)
    case line(Tone, String)
    /// The screen was cleared.
    case clear
    case status(String)
    case prompt(String)
    case unknown(String)
}

/// Phone → Tab5.
enum Outgoing: Equatable {
    /// "A": the HMAC from `TerminalProtocol.authAnswer`.
    case auth(String)
    /// "C": a command line, as if typed + Enter.
    case command(String)
    /// "K": a key the text field can't type.
    case key(RemoteKey)
}

enum RemoteKey: String {
    case esc
    case ctrlC = "ctrl-c"
    case clear
}

/// Notifications arrive cut to the MTU, not to lines: collect the bytes
/// and hand out whole lines. Splitting the bytes at "\n" before decoding
/// also keeps a UTF-8 character that straddles two chunks intact.
struct LineAssembler {
    /// More than this without a newline is garbage, not a line: dropped.
    static let maxPending = 16 * 1024

    private var pending = Data()

    mutating func feed(_ chunk: Data) -> [String] {
        pending.append(chunk)
        var lines: [String] = []
        while let newline = pending.firstIndex(of: 0x0A) {
            var raw = pending[pending.startIndex ..< newline]
            if raw.last == 0x0D { raw = raw.dropLast() }
            // Like the firmware: invalid UTF-8 becomes U+FFFD, not an error.
            lines.append(String(decoding: raw, as: UTF8.self))
            pending.removeSubrange(pending.startIndex ... newline)
        }
        if pending.count > Self.maxPending { pending.removeAll() }
        return lines
    }

    mutating func reset() {
        pending.removeAll()
    }
}
