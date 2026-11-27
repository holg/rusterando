# Tab5 Terminal

A small iPhone app that becomes the keyboard and screen of an M5Stack Tab5
running the lab firmware (`crates/tab5-lab`), over Bluetooth LE. It mirrors
the Tab5's terminal (coloured lines, status bar, prompt) and types into its
shell. Useful for a Tab5 that has no keyboard attached.

SwiftUI, iOS 16+, CoreBluetooth + CryptoKit, no dependencies.

## Build and run

```sh
cd ios-lab
cp project.local.example.yml project.local.yml   # once: your Team ID + bundle id
USE_LOCAL=true xcodegen
open Tab5Terminal.xcodeproj
```

Pick your iPhone as the run destination and press Run. Before the first
run from Xcode, turn on Developer Mode on the iPhone (Settings › Privacy &
Security › Developer Mode; it restarts).

Without `USE_LOCAL=true`, `xcodegen` builds the public project: placeholder
bundle id `com.example.tab5terminal`, no team. That's enough for the
simulator, but the simulator has no Bluetooth, so it only shows the device
list and runs the tests:

```sh
xcodebuild test -project Tab5Terminal.xcodeproj -scheme Tab5Terminal \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro'
```

## First use

1. Allow Bluetooth when asked.
2. Tap the gear and enter the shared key: `LAB_KEY` in `tmp/tab5-wifi.env`
   in the main checkout. It's the same key `scripts/build_tab5_lab.sh`
   compiles into every Tab5. With Universal Clipboard you can copy it on
   the Mac and paste it on the iPhone:
   `sed -n 's/^LAB_KEY=//p' tmp/tab5-wifi.env | pbcopy`.
   The key is stored in the iPhone's Keychain and never sent over the air.
3. Tap your Tab5 (`tab5-xxxxxx`) in the list.

The app remembers the last Tab5 and connects to it on launch. Turn that off
in Settings. Going back to the list disconnects.

## Using it

- **Status bar** (blue) and **prompt** (green) come from the Tab5.
- **Return** sends the line. ↑/↓ browse the history of sent lines.
- **Esc**, **Ctrl-C** and **Clear** are keys the text field can't type.
  `chat` enters LoRa chat mode (every line goes out over LoRa); Esc leaves it.
- Scroll up to read and the screen stops following. The ↓ button jumps back down.
- ⋯ › **Copy screen** copies the whole scrollback (last 1000 lines).
- A wrong or missing key opens the key sheet. A dropped link shows **Reconnect**.

## Protocol

This is the Nordic UART Service: the phone writes to RX `6E400002-…` and
subscribes to TX `6E400003-…`. Messages are UTF-8 lines ending in `\n`,
and the first character says what kind each one is. The source of truth is
[`crates/tab5-lab/src/remote.rs`](../crates/tab5-lab/src/remote.rs).

```
Tab5 → phone
  H <name> <version> <nonce>  hello on each connection; nonce: 32 hex digits
  O                           key accepted
  E <text>                    refused / error (wrong key, log in first, …)
  L<tone> <text>              screen line; tone n d e o r t w x
  X                           screen cleared
  S <text>                    status bar
  P <text>                    prompt
phone → Tab5
  A <hmac>                    HMAC-SHA256(key, nonce bytes), lowercase hex
  C <line>                    command line, as typed + Enter
  K <key>                     esc | ctrl-c | clear
```

After `O`, the Tab5 replays its last ~40 screen lines, then sends `S` and
`P`, then everything live.

## Code

| File | What |
| --- | --- |
| `Sources/TerminalProtocol.swift` | parse / encode, the HMAC login, line reassembly. Pure, no Bluetooth |
| `Sources/BLEManager.swift` | CoreBluetooth: scanning, one connection, login, serialized writes, the mirrored screen |
| `Sources/KeyStore.swift` | the shared key in the Keychain |
| `Sources/CommandHistory.swift` | ↑/↓ history, kept across launches |
| `Sources/*View.swift`, `CommandField.swift`, `Theme.swift` | the UI; colours are the Tab5's own (`console.rs`) |
| `Tests/TerminalProtocolTests.swift` | parser and encoder tests on a real session transcript, the HMAC vector, reassembly |
