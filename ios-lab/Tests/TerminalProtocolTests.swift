import XCTest
@testable import Tab5Terminal

final class TerminalProtocolTests: XCTestCase {
    /// Tab5 → phone half of a session (Mac + bleak, firmware 0.1.0); name,
    /// address and nonce replaced with placeholders.
    private let transcript = [
        "H tab5-aabbcc 0.1.0 00112233445566778899aabbccddeeff",
        "O",
        "Le tab5> rak server",
        "Lo tab5-lab 0.1.0 on tab5-aabbcc — type help",
        "Ld LAN: W5500 (v4) — CS GPIO4 (bus 20), RST GPIO35 (bus 24)",
        "S  tab5-aabbcc  ·  LAN 192.0.2.184  ·  RAK server  ·  no keyboard  ·  8.36 V +294 mA  ·  up 0:00:39",
        "P tab5> ",
        "Le tab5> info",
        "Ld free heap 31187 KB, internal 260 KB",
    ]

    private let nonceHex = "00112233445566778899aabbccddeeff"

    // MARK: - Parsing

    func testParsesTranscript() throws {
        let nonce = try XCTUnwrap(TerminalProtocol.unhex(nonceHex))
        XCTAssertEqual(transcript.map(TerminalProtocol.parse), [
            .hello(name: "tab5-aabbcc", version: "0.1.0", nonce: nonce),
            .accepted,
            .line(.echo, "tab5> rak server"),
            .line(.ok, "tab5-lab 0.1.0 on tab5-aabbcc — type help"),
            .line(.dim, "LAN: W5500 (v4) — CS GPIO4 (bus 20), RST GPIO35 (bus 24)"),
            // The leading space is the firmware's; kept exactly.
            .status(" tab5-aabbcc  ·  LAN 192.0.2.184  ·  RAK server  ·  no keyboard  ·  8.36 V +294 mA  ·  up 0:00:39"),
            // The trailing space too.
            .prompt("tab5> "),
            .line(.echo, "tab5> info"),
            .line(.dim, "free heap 31187 KB, internal 260 KB"),
        ])
    }

    func testHelloNonce() throws {
        guard case let .hello(name, version, nonce) = TerminalProtocol.parse(transcript[0]) else {
            return XCTFail("not a hello")
        }
        XCTAssertEqual(name, "tab5-aabbcc")
        XCTAssertEqual(version, "0.1.0")
        XCTAssertEqual(nonce.count, 16)
        XCTAssertEqual(nonce.first, 0x00)
        XCTAssertEqual(nonce.last, 0xFF)
        XCTAssertEqual(TerminalProtocol.hex(nonce), nonceHex)
    }

    func testBadHelloIsUnknown() {
        XCTAssertEqual(TerminalProtocol.parse("H tab5 0.1.0 0011"), .unknown("H tab5 0.1.0 0011"))
        XCTAssertEqual(TerminalProtocol.parse("H tab5 0.1.0 zz"), .unknown("H tab5 0.1.0 zz"))
        XCTAssertEqual(TerminalProtocol.parse("H"), .unknown("H"))
    }

    func testEveryTone() {
        let letters: [Character: Tone] = [
            "n": .normal, "d": .dim, "e": .echo, "o": .ok,
            "r": .received, "t": .sent, "w": .warning, "x": .error,
        ]
        for (letter, tone) in letters {
            XCTAssertEqual(TerminalProtocol.parse("L\(letter) hi"), .line(tone, "hi"))
        }
        XCTAssertEqual(Tone.allCases.count, letters.count)
    }

    func testLineEdgeCases() {
        XCTAssertEqual(TerminalProtocol.parse("Ln "), .line(.normal, ""))
        XCTAssertEqual(TerminalProtocol.parse("Ln"), .line(.normal, ""))
        // Indentation (help text) survives.
        XCTAssertEqual(TerminalProtocol.parse("Ln   rak ping [n]"), .line(.normal, "  rak ping [n]"))
        // A letter we don't know yet: still show the text.
        XCTAssertEqual(TerminalProtocol.parse("Lq new tone"), .line(.normal, "new tone"))
    }

    func testOtherMessages() {
        XCTAssertEqual(TerminalProtocol.parse("X"), .clear)
        XCTAssertEqual(TerminalProtocol.parse("E wrong key"), .error("wrong key"))
        XCTAssertEqual(TerminalProtocol.parse("E log in first"), .error("log in first"))
        XCTAssertEqual(TerminalProtocol.parse("E unknown key f1"), .error("unknown key f1"))
        XCTAssertEqual(TerminalProtocol.parse("P 192.168.1.5:5000> "), .prompt("192.168.1.5:5000> "))
        XCTAssertEqual(TerminalProtocol.parse("P lora> "), .prompt("lora> "))
        XCTAssertEqual(TerminalProtocol.parse("Z what"), .unknown("Z what"))
        XCTAssertEqual(TerminalProtocol.parse(""), .unknown(""))
    }

    // MARK: - Encoding

    func testEncodes() {
        XCTAssertEqual(text(.command("info")), "C info\n")
        XCTAssertEqual(text(.command("rak ping 3")), "C rak ping 3\n")
        // Empty: Enter on its own.
        XCTAssertEqual(text(.command("")), "C \n")
        XCTAssertEqual(text(.key(.esc)), "K esc\n")
        XCTAssertEqual(text(.key(.ctrlC)), "K ctrl-c\n")
        XCTAssertEqual(text(.key(.clear)), "K clear\n")
        XCTAssertEqual(text(.auth("00ff")), "A 00ff\n")
    }

    func testCommandStaysOneLine() {
        // A newline inside would start a second (unknown) message.
        XCTAssertEqual(text(.command("rak send hi\nthere")), "C rak send hi there\n")
        XCTAssertEqual(text(.command("a\r\nb")), "C a b\n")
    }

    func testCommandIsUTF8() {
        XCTAssertEqual(
            Array(TerminalProtocol.encode(.command("grüß"))),
            Array("C grüß\n".utf8)
        )
    }

    // MARK: - Login

    func testAuthAnswerKnownVector() throws {
        // python3: hmac.new(b'abcd-efgh-ijkm-npqr', bytes.fromhex(nonce), hashlib.sha256).hexdigest()
        let nonce = try XCTUnwrap(TerminalProtocol.unhex(nonceHex))
        let answer = TerminalProtocol.authAnswer(key: "abcd-efgh-ijkm-npqr", nonce: nonce)
        XCTAssertEqual(answer, "59e25ee55bd72010a44ff58da465a2c2c4086b7c42de2e803e468fa83cef6b35")
        XCTAssertEqual(answer.count, 64)
    }

    func testLoginFromHello() {
        guard case let .hello(_, _, nonce) = TerminalProtocol.parse(transcript[0]) else {
            return XCTFail("not a hello")
        }
        let answer = TerminalProtocol.authAnswer(key: "abcd-efgh-ijkm-npqr", nonce: nonce)
        XCTAssertEqual(
            text(.auth(answer)),
            "A 59e25ee55bd72010a44ff58da465a2c2c4086b7c42de2e803e468fa83cef6b35\n"
        )
    }

    func testHex() {
        XCTAssertEqual(TerminalProtocol.hex(Data([0x00, 0x0F, 0xAB, 0xFF])), "000fabff")
        XCTAssertEqual(TerminalProtocol.unhex("000FabFF"), Data([0x00, 0x0F, 0xAB, 0xFF]))
        XCTAssertEqual(TerminalProtocol.unhex(""), Data())
        XCTAssertNil(TerminalProtocol.unhex("abc"))
        XCTAssertNil(TerminalProtocol.unhex("zz"))
    }

    // MARK: - Reassembly

    func testLineSplitOverTwoChunks() {
        var assembler = LineAssembler()
        XCTAssertEqual(assembler.feed(Data("Ld free heap 311".utf8)), [])
        XCTAssertEqual(assembler.feed(Data("87 KB\n".utf8)), ["Ld free heap 31187 KB"])
    }

    func testTwoLinesInOneChunk() {
        var assembler = LineAssembler()
        XCTAssertEqual(assembler.feed(Data("O\nP tab5> \nLn par".utf8)), ["O", "P tab5> "])
        XCTAssertEqual(assembler.feed(Data("tial\n".utf8)), ["Ln partial"])
    }

    func testUTF8CharacterSplitAcrossChunks() {
        let bytes = Array("Lo on tab5 — type help\n".utf8)
        // "—" is E2 80 94: cut between its first and second byte.
        let dash = bytes.firstIndex(of: 0xE2)!
        var assembler = LineAssembler()
        XCTAssertEqual(assembler.feed(Data(bytes[..<(dash + 1)])), [])
        XCTAssertEqual(assembler.feed(Data(bytes[(dash + 1)...])), ["Lo on tab5 — type help"])
    }

    func testCarriageReturnAndEmptyLines() {
        var assembler = LineAssembler()
        XCTAssertEqual(assembler.feed(Data("X\r\n\nLn \n".utf8)), ["X", "", "Ln "])
    }

    func testTranscriptInMTUSizedChunks() {
        // As the notifications come: cut every 20 bytes (the smallest MTU),
        // regardless of lines and characters.
        let stream = Array(transcript.map { $0 + "\n" }.joined().utf8)
        var assembler = LineAssembler()
        var lines: [String] = []
        for start in stride(from: 0, to: stream.count, by: 20) {
            lines += assembler.feed(Data(stream[start ..< min(start + 20, stream.count)]))
        }
        XCTAssertEqual(lines, transcript)
    }

    func testRunawayLineIsDropped() {
        var assembler = LineAssembler()
        XCTAssertEqual(assembler.feed(Data(repeating: 0x41, count: LineAssembler.maxPending + 1)), [])
        // The garbage is gone; the next line starts clean.
        XCTAssertEqual(assembler.feed(Data("O\n".utf8)), ["O"])
    }

    func testReset() {
        var assembler = LineAssembler()
        _ = assembler.feed(Data("Ln half".utf8))
        assembler.reset()
        XCTAssertEqual(assembler.feed(Data("O\n".utf8)), ["O"])
    }

    // MARK: - Helpers

    private func text(_ message: Outgoing) -> String {
        String(decoding: TerminalProtocol.encode(message), as: UTF8.self)
    }
}

final class CommandHistoryTests: XCTestCase {
    func testBrowsesLikeAShell() {
        var history = CommandHistory(entries: [])
        XCTAssertNil(history.older())
        history.add("info")
        history.add("rak ping")
        history.add("rak ping") // repeat: kept once
        history.add("   ")      // blank: not kept
        XCTAssertEqual(history.entries, ["info", "rak ping"])

        XCTAssertEqual(history.older(), "rak ping")
        XCTAssertEqual(history.older(), "info")
        XCTAssertEqual(history.older(), "info") // stops at the oldest
        XCTAssertEqual(history.newer(), "rak ping")
        XCTAssertEqual(history.newer(), "")     // past the newest: empty line
        XCTAssertNil(history.newer())           // not browsing any more
    }

    func testKeepsTheLastHundred() {
        var history = CommandHistory(entries: [])
        for i in 0 ..< 150 { history.add("cmd \(i)") }
        XCTAssertEqual(history.entries.count, CommandHistory.limit)
        XCTAssertEqual(history.entries.first, "cmd 50")
        XCTAssertEqual(history.entries.last, "cmd 149")
    }
}
