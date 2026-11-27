import CoreBluetooth
import Foundation

/// A Tab5 heard while scanning.
struct Tab5: Identifiable, Equatable {
    let id: UUID
    var name: String
    var rssi: Int
}

/// The Tab5 the terminal talks to; also remembered as the last device.
struct Target: Equatable, Codable {
    let id: UUID
    var name: String
}

/// One line of the mirrored screen.
struct ScreenLine: Identifiable, Equatable {
    let id: Int
    let tone: Tone
    let text: String
}

/// The Bluetooth side: scans for Tab5s, holds one connection, logs in and
/// keeps the mirrored screen. CoreBluetooth calls back on the main queue,
/// so everything here runs on the main actor.
@MainActor
final class BLEManager: NSObject, ObservableObject {
    enum Phase: Equatable {
        /// Bluetooth off, not allowed, or missing; the reason.
        case unavailable(String)
        case idle
        case scanning
        case connecting
        /// Connected, waiting for the hello and then the key's answer.
        case waitingForKey
        /// No key saved, or the Tab5 refused it.
        case needsKey(String)
        /// Logged in: the terminal is live.
        case connected
        /// Dropped; the reason. The screen stays, Reconnect brings it back.
        case disconnected(String)

        var isDisconnected: Bool {
            if case .disconnected = self { return true }
            return false
        }
    }

    /// Scrollback kept on the phone.
    static let maxLines = 1000

    @Published private(set) var phase: Phase = .idle
    @Published private(set) var nearby: [Tab5] = []
    /// Non-nil while the terminal is open.
    @Published private(set) var target: Target?
    @Published private(set) var lastDevice: Target?
    @Published private(set) var lines: [ScreenLine] = []
    @Published private(set) var status = ""
    @Published private(set) var prompt = ""

    private let serviceID = CBUUID(string: TerminalProtocol.serviceUUID)
    private let rxID = CBUUID(string: TerminalProtocol.rxUUID)
    private let txID = CBUUID(string: TerminalProtocol.txUUID)

    private var central: CBCentralManager!
    private var peripheral: CBPeripheral?
    private var rx: CBCharacteristic?
    private var assembler = LineAssembler()
    private var nonce: Data?
    /// Chunks waiting to be written; one write outstanding at a time, so
    /// lines never interleave.
    private var outbox: [Data] = []
    private var writing = false

    private struct Heard {
        let peripheral: CBPeripheral
        var device: Tab5
        var at: Date
    }
    private var heard: [UUID: Heard] = [:]
    private var refresh: Task<Void, Never>?
    private var triedAutoConnect = false
    private var nextLineID = 0
    /// Whose lines are on screen: kept across reconnects to the same Tab5.
    private var screenOwner: UUID?

    override init() {
        super.init()
        if let data = UserDefaults.standard.data(forKey: Defaults.lastDevice) {
            lastDevice = try? JSONDecoder().decode(Target.self, from: data)
        }
        central = CBCentralManager(delegate: self, queue: nil)
    }

    // MARK: - Actions

    func connect(to target: Target) {
        if screenOwner != target.id {
            lines.removeAll()
            status = ""
            prompt = ""
            screenOwner = target.id
        }
        self.target = target
        remember(target)
        closeLink()
        guard central.state == .poweredOn else {
            phase = .disconnected(Self.reason(for: central.state) ?? "Bluetooth is not ready")
            return
        }
        phase = .connecting
        note("connecting to \(target.name)…")
        // Known to iOS (heard just now, or connected before): connect right
        // away — it waits until the Tab5 is in range. Otherwise scan for it.
        if let p = heard[target.id]?.peripheral
            ?? central.retrievePeripherals(withIdentifiers: [target.id]).first {
            open(p)
        }
        updateScan()
    }

    func connect(to device: Tab5) {
        connect(to: Target(id: device.id, name: device.name))
    }

    func reconnect() {
        if let target { connect(to: target) }
    }

    /// Leave the terminal: back to the device list.
    func disconnect() {
        closeLink()
        target = nil
        phase = listPhase()
        updateScan()
    }

    /// A command line, as if typed + Enter.
    func send(_ line: String) {
        guard phase == .connected else { return }
        transmit(.command(line))
    }

    func press(_ key: RemoteKey) {
        guard phase == .connected else { return }
        transmit(.key(key))
    }

    /// The key was changed in Settings: try it if the Tab5 is waiting.
    func keyChanged() {
        if case .needsKey = phase { login() }
    }

    func forgetLastDevice() {
        lastDevice = nil
        UserDefaults.standard.removeObject(forKey: Defaults.lastDevice)
    }

    // MARK: - Link

    private func open(_ p: CBPeripheral) {
        peripheral = p
        p.delegate = self
        central.connect(p)
    }

    /// Drop the connection (if any) and everything that belongs to it.
    private func closeLink() {
        if let p = peripheral, p.state != .disconnected {
            central.cancelPeripheralConnection(p)
        }
        peripheral = nil
        rx = nil
        nonce = nil
        outbox.removeAll()
        writing = false
        assembler.reset()
    }

    private func fail(_ reason: String) {
        closeLink()
        phase = .disconnected(reason)
        note("disconnected: \(reason)")
        updateScan()
    }

    private func login() {
        guard let nonce else { return }
        guard let key = KeyStore.load(), !key.isEmpty else {
            phase = .needsKey("Enter the shared key")
            return
        }
        phase = .waitingForKey
        transmit(.auth(TerminalProtocol.authAnswer(key: key, nonce: nonce)))
    }

    private func transmit(_ message: Outgoing) {
        guard let p = peripheral, rx != nil else { return }
        let data = TerminalProtocol.encode(message)
        let size = max(20, p.maximumWriteValueLength(for: .withResponse))
        var start = 0
        while start < data.count {
            let end = min(start + size, data.count)
            outbox.append(data.subdata(in: start ..< end))
            start = end
        }
        pump()
    }

    /// Next chunk, once the last one was acknowledged.
    private func pump() {
        guard !writing, !outbox.isEmpty, let p = peripheral, let rx else { return }
        writing = true
        p.writeValue(outbox.removeFirst(), for: rx, type: .withResponse)
    }

    private func handle(_ message: Incoming) {
        switch message {
        case let .hello(name, version, nonce):
            self.nonce = nonce
            if var t = target, t.name != name {
                t.name = name
                target = t
                remember(t)
            }
            note("── \(name) \(version) ──")
            login()
        case .accepted:
            phase = .connected
        case .error(let text):
            append(.error, text)
            // Any error while the key is being checked: the login failed
            // ("wrong key", or a Tab5 built without one).
            if phase == .waitingForKey || text == "wrong key" {
                phase = .needsKey(text == "wrong key" ? "Wrong key" : text)
            }
        case let .line(tone, text):
            append(tone, text)
        case .clear:
            lines.removeAll()
        case .status(let text):
            status = text.trimmingCharacters(in: .whitespaces)
        case .prompt(let text):
            prompt = text
        case .unknown:
            break
        }
    }

    // MARK: - Screen

    private func append(_ tone: Tone, _ text: String) {
        lines.append(ScreenLine(id: nextLineID, tone: tone, text: text))
        nextLineID += 1
        if lines.count > Self.maxLines {
            lines.removeFirst(lines.count - Self.maxLines)
        }
    }

    /// The app's own remarks, dim, in between the Tab5's lines.
    private func note(_ text: String) {
        append(.dim, text)
    }

    // MARK: - Scanning

    /// Scan while the device list shows, or for a target iOS didn't know.
    private func updateScan() {
        let wanted = central.state == .poweredOn
            && (target == nil || (phase == .connecting && peripheral == nil))
        if wanted, !central.isScanning {
            // Duplicates on: every advertisement updates the RSSI.
            central.scanForPeripherals(
                withServices: [serviceID],
                options: [CBCentralManagerScanOptionAllowDuplicatesKey: true]
            )
            refresh = Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(nanoseconds: 1_000_000_000)
                    self?.publishNearby()
                }
            }
        } else if !wanted, central.isScanning {
            central.stopScan()
            refresh?.cancel()
            refresh = nil
        }
    }

    /// Once a second, not per advertisement (dozens a second): forget the
    /// quiet ones, sort by name so rows don't jump around.
    private func publishNearby() {
        let cutoff = Date().addingTimeInterval(-6)
        heard = heard.filter { $0.value.at > cutoff }
        nearby = heard.values.map(\.device).sorted {
            ($0.name, $0.id.uuidString) < ($1.name, $1.id.uuidString)
        }
    }

    private func listPhase() -> Phase {
        if central.state == .poweredOn { return .scanning }
        return Self.reason(for: central.state).map(Phase.unavailable) ?? .idle
    }

    private static func reason(for state: CBManagerState) -> String? {
        switch state {
        case .poweredOff: return "Bluetooth is off"
        case .unauthorized: return "Bluetooth not allowed — enable it in Settings › Tab5 Terminal"
        case .unsupported: return "No Bluetooth LE on this device"
        default: return nil
        }
    }

    private func remember(_ target: Target) {
        lastDevice = target
        if let data = try? JSONEncoder().encode(target) {
            UserDefaults.standard.set(data, forKey: Defaults.lastDevice)
        }
    }
}

enum Defaults {
    static let lastDevice = "lastDevice"
    /// Read once at launch; Settings writes it through @AppStorage.
    static let autoConnect = "autoConnect"
}

// MARK: - CoreBluetooth

extension BLEManager: @preconcurrency CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        if central.state != .poweredOn {
            heard.removeAll()
            nearby = []
            // iOS drops the link without a disconnect callback.
            if target != nil, !phase.isDisconnected {
                closeLink()
                phase = .disconnected(Self.reason(for: central.state) ?? "Bluetooth went away")
            }
        }
        if target == nil { phase = listPhase() }

        if central.state == .poweredOn, !triedAutoConnect {
            triedAutoConnect = true
            let auto = UserDefaults.standard.object(forKey: Defaults.autoConnect) as? Bool ?? true
            if auto, target == nil, let last = lastDevice { connect(to: last) }
        }
        updateScan()
    }

    func centralManager(
        _ central: CBCentralManager,
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String: Any],
        rssi: NSNumber
    ) {
        let id = peripheral.identifier
        let before = heard[id]
        // The name comes in the scan response, so not with every packet.
        let name = advertisementData[CBAdvertisementDataLocalNameKey] as? String
            ?? peripheral.name ?? before?.device.name ?? "Tab5"
        // 127 means "no reading".
        let level = rssi.intValue == 127 ? (before?.device.rssi ?? -100) : rssi.intValue
        heard[id] = Heard(peripheral: peripheral, device: Tab5(id: id, name: name, rssi: level), at: Date())
        if before == nil { publishNearby() }

        // The target iOS couldn't retrieve: here it is.
        if target?.id == id, phase == .connecting, self.peripheral == nil {
            open(peripheral)
            updateScan()
        }
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        guard peripheral == self.peripheral else { return }
        peripheral.discoverServices([serviceID])
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
        guard peripheral == self.peripheral else { return }
        fail(error?.localizedDescription ?? "couldn't connect")
    }

    func centralManager(
        _ central: CBCentralManager,
        didDisconnectPeripheral peripheral: CBPeripheral,
        error: Error?
    ) {
        // No error: we hung up ourselves, already handled.
        guard let error, peripheral == self.peripheral else { return }
        fail(error.localizedDescription)
    }
}

extension BLEManager: @preconcurrency CBPeripheralDelegate {
    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard peripheral == self.peripheral else { return }
        guard let service = peripheral.services?.first(where: { $0.uuid == serviceID }) else {
            fail(error?.localizedDescription ?? "no terminal service on this device")
            return
        }
        peripheral.discoverCharacteristics([rxID, txID], for: service)
    }

    func peripheral(
        _ peripheral: CBPeripheral,
        didDiscoverCharacteristicsFor service: CBService,
        error: Error?
    ) {
        guard peripheral == self.peripheral else { return }
        let found = service.characteristics ?? []
        guard let rx = found.first(where: { $0.uuid == rxID }),
              let tx = found.first(where: { $0.uuid == txID })
        else {
            fail(error?.localizedDescription ?? "terminal service incomplete")
            return
        }
        self.rx = rx
        // The Tab5 says hello once notifications are on.
        peripheral.setNotifyValue(true, for: tx)
    }

    func peripheral(
        _ peripheral: CBPeripheral,
        didUpdateNotificationStateFor characteristic: CBCharacteristic,
        error: Error?
    ) {
        guard peripheral == self.peripheral else { return }
        if let error {
            fail(error.localizedDescription)
        } else if characteristic.isNotifying, phase == .connecting {
            phase = .waitingForKey
        }
    }

    func peripheral(
        _ peripheral: CBPeripheral,
        didUpdateValueFor characteristic: CBCharacteristic,
        error: Error?
    ) {
        guard peripheral == self.peripheral, error == nil, let data = characteristic.value else { return }
        for line in assembler.feed(data) {
            handle(TerminalProtocol.parse(line))
        }
    }

    func peripheral(
        _ peripheral: CBPeripheral,
        didWriteValueFor characteristic: CBCharacteristic,
        error: Error?
    ) {
        guard peripheral == self.peripheral else { return }
        writing = false
        if let error { append(.error, "write failed: \(error.localizedDescription)") }
        pump()
    }
}
