import SwiftUI

@main
struct Tab5TerminalApp: App {
    @StateObject private var ble = BLEManager()

    var body: some Scene {
        WindowGroup {
            NavigationStack {
                DeviceListView()
                    // The terminal is open while there is a target; going
                    // back hangs up.
                    .navigationDestination(isPresented: Binding(
                        get: { ble.target != nil },
                        set: { if !$0 { ble.disconnect() } }
                    )) {
                        TerminalView()
                    }
            }
            .environmentObject(ble)
            .preferredColorScheme(.dark)
            .tint(Tone.ok.color)
        }
    }
}
