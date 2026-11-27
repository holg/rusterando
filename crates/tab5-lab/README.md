# tab5-lab — Tab5 Keyboard, LAN and LoRa samples

Sample firmware for the M5Stack Tab5 with its add-ons, stacked as on both
of our Tab5s: Module13.2 LAN on the Tab5, Module LoRaWAN-EU868 on top.

| Add-on | Connection | Sample |
|---|---|---|
| Tab5 Keyboard (A164) | EXT header: I2C 0x6D, SDA GPIO0, SCL GPIO1, INT GPIO50 | 80×24 text terminal on the screen |
| Module13.2 LAN (W5500) | M5-Bus SPI | LAN client: DHCP, ping, HTTP(S) GET, TCP terminal |
| Module LoRaWAN-EU868 (M148, RAK3172) | M5-Bus UART, 115200 | LoRa P2P listen / server / chat / ping, AT commands from the shell |

Rust std on ESP-IDF 5.4.4, like the kitchen firmware (`crates/kds-tab5`),
whose board setup (I/O expanders, charging) and screen turning it reuses.
The kitchen firmware is not changed.

```sh
PROFILE=release scripts/build_tab5_lab.sh flash     # build, flash, show the log
PROFILE=release LAB_AUTORUN="rak server" scripts/build_tab5_lab.sh flash
                                                    # a keyboard-less LoRa server
PROFILE=release scripts/build_kds_tab5.sh flash     # back to the kitchen screen
```

## Commands

`help` on the device. In short:

- `info` — modules found, their pins, keyboard
- `lan` / `lan restart` / `ping <host>` / `get <url>` / `tcp <host:port>`
- `rak` / `rak listen|server|stop` / `rak client` (or `chat`) /
  `rak send <text>` / `rak ping [n]` / `at+<cmd>` (any AT command)
- `flip` turns the screen round; Esc or Ctrl+C leaves chat and TCP;
  Ctrl+L clears; Up/Down history

The bus 5 V (SYS_EXT5VO, M5-Bus pin 28 and the EXT header) powers all
three add-ons. It is switched by I/O expander 0x43 P2; this firmware
switches it on (the kitchen firmware leaves it off).

## LAN module (W5500)

SPI: SCK GPIO5 (bus 11), MOSI GPIO18 (bus 7), MISO GPIO19 (bus 9), 8 MHz.
CS and RST are set with jumper caps; the firmware finds them at start
(`info`):

| bus pin | Tab5 GPIO | jumper |
|---|---|---|
| 20 | 4  | CS (as delivered) |
| 23 | 47 | CS |
| 24 | 35 | RST (as delivered; also the P4's boot strap pin) |
| 22 | 48 | RST |
| 2 / 26 | 16 / 51 | INT (not used: the W5500 is polled) |

Test against a Mac:

```sh
nc -l 5000                      # on the Mac
tcp 192.0.2.20:5000             # on the Tab5 (the Mac's IP)
```

Lines typed on either side show up on the other. Esc on the Tab5 closes
the connection.

ESP-IDF 5.4's W5500 driver never recovers from one garbled frame header:
it logs "received frame was truncated" hundreds of times a second and its
task holds a CPU (the kitchen screen stopped reacting to touch). src/lan.rs
counts those log lines and restarts the driver when they come in a burst
(after 3 restarts in 10 minutes, the Tab5). The SPI clock went from 16 to
8 MHz to make the garbled reads rarer.

## LoRaWAN module (RAK3172)

Module LoRaWAN-EU868 (M5Stack M148): a RAK3172-8-SM-I with RAK's RUI3 AT
firmware on a UART (115200 8N1), 3.3 V from its own regulator on the bus
5 V. A 12-way DIP switch picks the bus pins; TX/RX are named from the
host's side (schematic `SCH_LoRaWAN868_HW1.0`):

| DIP | line | Core pin | bus | Tab5 GPIO |
|---|---|---|---|---|
| 1 | TX | G1  | 14 | 37 — UART0, the console: don't |
| 2 | TX | G17 | 16 | 6 |
| 3 | TX | G15 | 23 | 47 (a LAN CS position) |
| 4 | TX | G12 | 21 | 2 |
| 5 | TX | G0  | 24 | 35 (the LAN reset as delivered) |
| 6 | RX | G3  | 13 | 38 — UART0, the console: don't |
| 7 | RX | G16 | 15 | 7 |
| 8 | RX | G13 | 22 | 48 |
| 9 | RX | G34 | 26 | 51 |
| 10 | RX | G35 | 2 | 16 |
| 11 | RST | G13 | 22 | 48 — never with 8 (TX on its own reset) |
| 12 | RST | G25 | 8 | 45 |

One TX and one RX switch ON; no RST switch needed (the module pulls its
reset up). On both Tab5s: **DIP 2 + 8** (TX bus 16, RX bus 22) — clear of
the LAN module's pins. The probe finds the RX line by its idle-high level,
then sends `AT` on each TX position until one answers `OK`.

### P2P client/server

Default 869.525 MHz, SF7, 125 kHz, CR 4/5, preamble 8, 14 dBm, set at
start in P2P mode (`at+nwm=0` switches a module to P2P; it restarts).
869.4–869.65 MHz allows 500 mW ERP at 10 % duty cycle, so 14 dBm leaves
plenty of room. Both ends must use the same settings.

On air, plain text, so other LoRa gear can take part:

```text
T5 MSG  <seq> <from> <text>
T5 ACK  <seq> <from> <rssi> <snr>      (as the answering end heard it)
T5 PING <seq> <from>
T5 PONG <seq> <from> <rssi> <snr>
```

Two Tab5s: one `rak server` (or built with `LAB_AUTORUN="rak server"`),
the other `rak ping` — or both `chat` and type away. Every ack shows the
round trip and the signal at both ends.

Other settings by hand, e.g. `at+p2p=869525000:9:125:0:8:14`
(Hz:SF:BW:CR:preamble:dBm); `at+p2p=?` reads them back (some RUI versions
show the bandwidth as an index, 0 = 125 kHz).

## Phone as terminal (Bluetooth LE)

The Tab5's Bluetooth runs on its ESP32-C6, reached through esp_hosted
(the same SDIO link as Wi-Fi; M5Stack's factory C6 firmware has it). The
firmware advertises its name (`tab5-xxxxxx`) with the Nordic UART Service
(components/lab_ble, C) and mirrors its screen to a phone that logs in:
src/remote.rs has the line protocol. The login is a challenge answered
with HMAC-SHA256 of the shared `LAB_KEY` (from `tmp/tab5-wifi.env`; the
build script makes one if missing); the key never goes over the air.

The iPhone app is in `ios-lab/` (Tab5 Terminal). Desk test from a Mac:
`tmp/ble-venv/bin/python -I tmp/ble_term_test.py tab5-aabbcc info`.

## Admin page: LoRa tab

Every LoRa packet a Tab sends, receives or gets no answer to goes to the
shop server (src/report.rs → `POST <SHOP_URL>/api/lab/lora`, bearer
`LAB_KEY`; no SHOP_URL in `tmp/tab5-wifi.env` → no reporting, the rest
of the lab works as before), which shows it at `/admin/lora` as a live
chat: who sent what, who heard it how well, how fast the answer came. Messages typed there wait
until the chosen Tab asks (every 3 s) and go out over LoRa from it. Server
side: crates/rusterando-server/src/lab_lora.rs, page
crates/rusterando-frontend/src/pages/admin/lora.rs, tables
migrations/20261110000001_lab_lora.sql. The server needs the same
`LAB_KEY` in its environment.
