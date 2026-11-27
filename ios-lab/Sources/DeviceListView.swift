import SwiftUI

/// Tab5s in range, strongest signal shown as bars; tap one to connect.
struct DeviceListView: View {
    @EnvironmentObject private var ble: BLEManager
    @State private var showSettings = false

    var body: some View {
        List {
            if case .unavailable(let why) = ble.phase {
                Section {
                    Label(why, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(Tone.warning.color)
                }
            }

            // Not in range right now: connecting still works, iOS waits for it.
            if let last = ble.lastDevice, !ble.nearby.contains(where: { $0.id == last.id }) {
                Section("Last device") {
                    Button { ble.connect(to: last) } label: {
                        HStack {
                            Text(last.name).font(.body.monospaced())
                            Spacer()
                            Text("Connect").foregroundStyle(.secondary)
                        }
                    }
                }
            }

            Section {
                if ble.nearby.isEmpty {
                    Text(ble.phase == .scanning ? "Looking for Tab5s…" : "Nothing found")
                        .foregroundStyle(.secondary)
                }
                ForEach(ble.nearby) { device in
                    Button { ble.connect(to: device) } label: { row(device) }
                }
            } header: {
                HStack(spacing: 6) {
                    Text("Nearby")
                    if ble.phase == .scanning { ProgressView().controlSize(.mini) }
                }
            }
        }
        .navigationTitle("Tab5 Terminal")
        .toolbar {
            Button { showSettings = true } label: { Image(systemName: "gearshape") }
                .accessibilityLabel("Settings")
        }
        .sheet(isPresented: $showSettings) { SettingsView().environmentObject(ble) }
    }

    private func row(_ device: Tab5) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(device.name).font(.body.monospaced()).foregroundStyle(.primary)
                if device.id == ble.lastDevice?.id {
                    Text("last used").font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer()
            Text("\(device.rssi) dBm")
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
            Image(systemName: "cellularbars", variableValue: Self.strength(device.rssi))
                .foregroundStyle(.secondary)
        }
    }

    /// -95 dBm (barely) … -55 dBm (next to it) as 0…1.
    private static func strength(_ rssi: Int) -> Double {
        min(1, max(0, Double(rssi + 95) / 40))
    }
}
