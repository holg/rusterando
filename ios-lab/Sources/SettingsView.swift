import SwiftUI

/// The shared key and auto-connect.
struct SettingsView: View {
    @EnvironmentObject private var ble: BLEManager
    @Environment(\.dismiss) private var dismiss
    /// Why the sheet came up by itself ("Wrong key"), if it did.
    var notice: String?

    @State private var key = ""
    @State private var loaded = false
    @State private var reveal = false
    @State private var saveFailed = false
    @AppStorage(Defaults.autoConnect) private var autoConnect = true

    var body: some View {
        NavigationStack {
            Form {
                if let notice {
                    Section {
                        Label(notice, systemImage: "key").foregroundStyle(Tone.error.color)
                    }
                }

                Section {
                    HStack {
                        Group {
                            if reveal {
                                TextField("xxxx-xxxx-xxxx-xxxx", text: $key)
                            } else {
                                SecureField("xxxx-xxxx-xxxx-xxxx", text: $key)
                            }
                        }
                        .font(.body.monospaced())
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit(save)
                        Button { reveal.toggle() } label: {
                            Image(systemName: reveal ? "eye.slash" : "eye")
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel(reveal ? "Hide key" : "Show key")
                    }
                } header: {
                    Text("Shared key")
                } footer: {
                    Text(saveFailed
                        ? "Couldn't save the key to the Keychain."
                        : "LAB_KEY of the Tab5 build, the same for all Tabs. Kept in this iPhone's Keychain.")
                }

                Section {
                    Toggle("Auto-connect on launch", isOn: $autoConnect)
                    if let last = ble.lastDevice {
                        LabeledContent("Last device") {
                            Text(last.name).font(.body.monospaced())
                        }
                        Button("Forget last device", role: .destructive) { ble.forgetLastDevice() }
                    }
                }

                Section {
                    LabeledContent("Version", value: Self.version)
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save)
                }
            }
            .onAppear {
                // Once: a re-render must not overwrite what's being typed.
                guard !loaded else { return }
                loaded = true
                key = KeyStore.load() ?? ""
            }
        }
    }

    private func save() {
        // Pasted keys often bring a space or newline along.
        guard KeyStore.save(key.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            saveFailed = true
            return
        }
        ble.keyChanged()
        dismiss()
    }

    private static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "?"
    }
}
