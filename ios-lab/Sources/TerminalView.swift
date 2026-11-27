import SwiftUI
import UIKit

/// The Tab5's screen and keyboard: status bar, mirrored lines, keys, prompt.
struct TerminalView: View {
    @EnvironmentObject private var ble: BLEManager
    @State private var input = ""
    @State private var history = CommandHistory.load()
    @State private var showSettings = false

    /// One tap each; `chat` is left with Esc.
    private static let quickCommands = ["help", "info", "rak ping", "chat"]

    var body: some View {
        VStack(spacing: 0) {
            if !ble.status.isEmpty { statusBar }
            banner
            ScreenView(lines: ble.lines)
            keyBar
            inputRow
        }
        .background(Theme.background.ignoresSafeArea())
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(Theme.background, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .principal) { title }
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button {
                        UIPasteboard.general.string = ble.lines.map(\.text).joined(separator: "\n")
                    } label: { Label("Copy screen", systemImage: "doc.on.doc") }
                    Button { showSettings = true } label: { Label("Settings", systemImage: "gearshape") }
                } label: { Image(systemName: "ellipsis.circle") }
            }
        }
        .sheet(isPresented: $showSettings) { SettingsView(notice: keyNotice).environmentObject(ble) }
        // No key, or a wrong one: ask for it.
        .onChange(of: ble.phase) { phase in
            if case .needsKey = phase { showSettings = true }
        }
        .onAppear {
            if case .needsKey = ble.phase { showSettings = true }
        }
    }

    private var keyNotice: String? {
        if case .needsKey(let why) = ble.phase { return why }
        return nil
    }

    private var title: some View {
        VStack(spacing: 1) {
            Text(ble.target?.name ?? "Tab5")
                .font(.subheadline.monospaced().weight(.semibold))
            HStack(spacing: 4) {
                Circle().fill(ble.phase.color).frame(width: 6, height: 6)
                Text(ble.phase.label).font(.caption2)
            }
            .foregroundStyle(ble.phase.color)
        }
    }

    private var statusBar: some View {
        Text(ble.status)
            .font(.system(size: 11, design: .monospaced))
            .foregroundStyle(Theme.barText)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .panel(Theme.barBackground)
    }

    /// What's going on while the terminal isn't live, and the way out.
    @ViewBuilder private var banner: some View {
        switch ble.phase {
        case .connecting:
            bannerRow("Connecting to \(ble.target?.name ?? "Tab5")…", tone: .sent, busy: true)
        case .waitingForKey:
            bannerRow("Logging in…", tone: .sent, busy: true)
        case .needsKey(let why):
            bannerRow(why, tone: .error, button: "Enter key") { showSettings = true }
        case .disconnected(let why):
            bannerRow("Disconnected: \(why)", tone: .error, button: "Reconnect") { ble.reconnect() }
        default:
            EmptyView()
        }
    }

    private func bannerRow(
        _ text: String,
        tone: Tone,
        busy: Bool = false,
        button: String? = nil,
        action: @escaping () -> Void = {}
    ) -> some View {
        HStack(spacing: 8) {
            if busy { ProgressView().controlSize(.small) }
            Text(text)
                .font(.footnote)
                .foregroundStyle(tone.color)
                .lineLimit(2)
            Spacer(minLength: 0)
            if let button {
                Button(button, action: action)
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .panel(Theme.panel)
    }

    private var keyBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                key("Esc") { ble.press(.esc) }
                key("Ctrl-C") { ble.press(.ctrlC) }
                key("Clear") { ble.press(.clear) }
                Divider().frame(height: 18)
                ForEach(Self.quickCommands, id: \.self) { command in
                    key(command) { ble.send(command) }
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
        }
        .panel(Theme.panel)
        .disabled(ble.phase != .connected)
    }

    private func key(_ title: String, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .font(.system(size: 13, design: .monospaced))
            .buttonStyle(.bordered)
            .controlSize(.small)
            .tint(Tone.normal.color)
    }

    private var inputRow: some View {
        HStack(spacing: 6) {
            Text(ble.prompt.isEmpty ? ">" : ble.prompt)
                .font(.system(size: 15, design: .monospaced))
                .foregroundStyle(Tone.ok.color)
                .lineLimit(1)
                .fixedSize()
            CommandField(text: $input, onSubmit: submit)
                .frame(height: 36)
            Button {
                if let line = history.older() { input = line }
            } label: { Image(systemName: "chevron.up") }
                .accessibilityLabel("Previous command")
            Button {
                if let line = history.newer() { input = line }
            } label: { Image(systemName: "chevron.down") }
                .accessibilityLabel("Next command")
        }
        .buttonStyle(.bordered)
        .controlSize(.small)
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .panel(Theme.panel)
    }

    private func submit() {
        // Not live: keep the line, Return again once connected.
        guard ble.phase == .connected else { return }
        ble.send(input)
        history.add(input)
        history.save()
        input = ""
    }
}

private extension View {
    /// Bars run edge to edge: into the side insets in landscape and under
    /// the home indicator (only where the bar touches those edges).
    func panel(_ color: Color) -> some View {
        background(color.ignoresSafeArea(.container, edges: [.horizontal, .bottom]))
    }
}
