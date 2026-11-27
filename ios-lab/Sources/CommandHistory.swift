import Foundation

/// Sent lines, browsed with ↑/↓ like a shell's history; kept across launches.
struct CommandHistory {
    static let limit = 100
    private static let defaultsKey = "history"

    private(set) var entries: [String]
    /// The entry shown while browsing; nil when not browsing.
    private var index: Int?

    init(entries: [String]) {
        self.entries = entries
    }

    static func load() -> CommandHistory {
        CommandHistory(entries: UserDefaults.standard.stringArray(forKey: defaultsKey) ?? [])
    }

    func save() {
        UserDefaults.standard.set(entries, forKey: Self.defaultsKey)
    }

    mutating func add(_ line: String) {
        index = nil
        guard !line.trimmingCharacters(in: .whitespaces).isEmpty, entries.last != line else { return }
        entries.append(line)
        if entries.count > Self.limit { entries.removeFirst(entries.count - Self.limit) }
    }

    /// ↑: the entry before the one shown, starting from the newest.
    mutating func older() -> String? {
        guard !entries.isEmpty else { return nil }
        let i = index.map { max(0, $0 - 1) } ?? entries.count - 1
        index = i
        return entries[i]
    }

    /// ↓: the entry after; past the newest an empty line, like a shell.
    mutating func newer() -> String? {
        guard let i = index else { return nil }
        if i + 1 < entries.count {
            index = i + 1
            return entries[i + 1]
        }
        index = nil
        return ""
    }
}
