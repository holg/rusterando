# Kitchen display — STM32MP135F-DK

One board replaces the Pi: receipt printer client, SSH tunnel, and a
480×272 touch board with three columns (**Neu / In Arbeit / Fertig**).
Same binary as the Pi printer (`rusterando-printer`), built with
`--features kds-kms`; same kitchen channel (kitchen-protocol 2.1+), so the
server needs nothing new beyond this branch.

| File | Goes to (on the board) |
|---|---|
| `rusterando-kds.service` | `/etc/systemd/system/` — display + printer, stops Weston |
| `rusterando-tunnel.service` | `/etc/systemd/system/` — SSH tunnel (OpenSSH if installed, else Dropbear `dbclient`) |
| `convert-tunnel-key.py` | `/usr/local/bin/` — Dropbear → OpenSSH key conversion, on the board |
| `rusterando-bt-keep` (+ `.service`, `.timer`) | `/usr/local/bin/`, `/etc/systemd/system/` — keeps the gong speaker connected |
| `99-rusterando-printer.rules` | `/etc/udev/rules.d/` — USB printer access for group `lp` |
| `printer.env.example` | `/etc/rusterando-printer/printer.env` |
| `tunnel.env.example` | `/etc/rusterando-printer/tunnel.env` |

Board facts that shaped this (OpenSTLinux 4.0 kirkstone, kernel 5.15):
no `usblp` in the kernel (so `PRINTER_PATH=usb` talks to the printer over
usbfs), Dropbear instead of OpenSSH, glibc 2.35, no GPU (Slint software
renderer straight to DRM/KMS), board clock in UTC (the unit sets
`TZ=Europe/Berlin`).

## First install

```bash
# from the repo root; builds for armv7, copies the board's libinput etc.
# into tmp/mp135-sysroot on first run
KDS_SSH_HOST=root@<board-ip> ./scripts/deploy_kds.sh --setup
```

`--setup` installs the units, the udev rule and both env files (existing
env files are kept), **disables Weston** and enables `rusterando-kds`.
Without a tunnel the screen shows "Verbinde mit dem Server …".

Try the UI without a server: `KDS_DEMO=1` in `printer.env` (sample
orders, moves applied locally).

## On the screen

Three pages: **Start** (left), **Küche** (middle, default), **Zeiten**
(right). Switch with the header tabs or a two-finger swipe (like turning
pages: fingers to the left → the page to the right). A new order, and
waking from the screensaver, always go to Küche.

- **Start** — clock, date, shop status, the three column counts (tap →
  Küche), utility buttons: Testdruck (sample receipt on the configured
  printer) and Globus (screensaver now). Info line: version, printer, LAN
  address.
- **Küche** — Neu / In Arbeit / Fertig; cards show BAR (red) or BEZAHLT
  (green). Tap a card for the order and its buttons (Starten, Fertig,
  Abgeholt / Fahrer unterwegs, Zurück; Storno at the bottom, two taps).
- **Zeiten** — the quick controls of `/admin/hours`, run by the same server
  code: Jetzt öffnen / schließen, Keine Lieferung, Pause 30 Min / 1 Std /
  2 Std, Auf Plan, Notbetrieb (only while customers can't order), heute
  früher / länger (±30 Min, ±1 Std, or a time via the ab / bis stepper).
  Refusals show the server's message in red. The shop chip in the header
  also opens it. Not on the terminal: the weekly plan and special days.
- **New-order alert** — the screen flashes white 3× with the order
  ("NEUE BESTELLUNG · #0012 · LIEF · BAR") and the gong rings on the paired
  Bluetooth speaker. The display writes `/run/rusterando-kds/gong`;
  `rusterando-gong.path` (root) plays `/usr/local/share/rusterando/gong.wav`
  via PipeWire, which runs as a lingering session of root (setup enables
  it). Pair a speaker once: `bluetoothctl` → `scan on` → `pair <MAC>` →
  `trust <MAC>` → `connect <MAC>`. `rusterando-bt-keep.timer` checks every
  minute: it powers the adapter on (after a reboot it sometimes comes up off)
  and reconnects every trusted audio device that dropped. PipeWire must run
  from boot (`pipewire.service` enabled in root's user session), otherwise
  nothing accepts the speaker until the first gong, and that first gong fails
  with "no target node available". Volume:
  `XDG_RUNTIME_DIR=/run/user/0 wpctl set-volume @DEFAULT_AUDIO_SINK@ 1.0`.
  New sound: `systemd/kds/make_gong.py`.
- **Screensaver** — after `KDS_SAVER_SECS` (default 5 min) without a touch
  and with nothing in "Neu", the screen goes to `KDS_SAVER_CMD` (any
  full-screen DRM program; unset = no saver). Single taps and drags
  belong to the saver; a **double tap** or a **new order** brings the board
  back. The display keeps running underneath (printing, board updates);
  only the screen is lent out (DRM master handover, `src/kds/saver.rs`).
  Don't start the saver by hand while `rusterando-kds` runs — it would find
  the screen taken; the display waits (instead of restart-looping) while
  something else holds the screen.

## Camera (`kamera`, copy to `/usr/local/bin/`)

`kamera foto` → `/tmp/kamera.jpg`; `kamera live [min]` → MJPEG on
`127.0.0.1:8081` (loopback only, view through SSH:
`ssh -L 8081:127.0.0.1:8081 root@<board> 'kamera live'` and
`ffplay -f mjpeg tcp://127.0.0.1:8081`). No H.264 encoder on the MP135, so
JPEG frames at 5 fps (~40 % CPU while it runs; stops after `min`, default 10).

## Tunnel

Two ways to give the board an identity the server accepts:

- **Own key (preferred)** — generated on the board, never leaves it, one
  `authorized_keys` line on the server (steps 1–2 below). Revocable
  without touching the Pi.
- **Reuse the Pi's key** — no server change. Dropbear can't read the
  OpenSSH key format, so convert it on the board:
  `dropbearconvert openssh dropbear <copy> /etc/rusterando-printer/tunnel_key`.
  Leave `TUNNEL_ADMIN_PORT` empty (the key's server entry is set up for the
  Pi's 2222).

1. Key, generated on the board (never leaves it):

   ```sh
   /usr/sbin/dropbearkey -t ed25519 -f /etc/rusterando-printer/tunnel_key | grep ^ssh-ed25519
   ```

2. On the server, add that public key to the tunnel user's
   `authorized_keys` with the same options as the Pi's line, but its own
   `permitlisten` port (the Pi holds 2222):

   ```
   restrict,port-forwarding,permitopen="127.0.0.1:9001",permitlisten="127.0.0.1:2223" ssh-ed25519 AAAA… kds@davidspizzeria
   ```

   Give the tunnel user `ClientAliveInterval 15` / `ClientAliveCountMax 3`
   in its sshd `Match` block if it doesn't have them yet — otherwise a
   board reboot leaves a dead session holding the `-R` port (the Pi's
   stale-2222 problem).

3. Pin the server host key once, interactively (writes
   `/home/root/.ssh/known_hosts`; answer `y` only if the fingerprint is
   `SHA256:2X/DScc0YOooEVVZfEEI/9GTHxyzBLAHCyrPi/RFWIM` — the one the Pi
   has pinned for `[davidspizzeria.de]:2200`):

   ```sh
   dbclient -p 2200 -i /etc/rusterando-printer/tunnel_key tunnel-davidspizzeria@davidspizzeria.de
   ```

4. `systemctl enable --now rusterando-tunnel`

Admin SSH into the board then goes through the server (once
`TUNNEL_ADMIN_PORT` is set): `ssh -p <port> root@127.0.0.1` on the server,
or from a laptop `ssh davidspizzeria-kds` (ProxyJump via the server to
`localhost:2222`, user root).

### OpenSSH instead of Dropbear

Installing OpenSSH on the board (it also comes with `apt-get install
autossh`) **removes Dropbear, including `dbclient`**. That is what took the
printer offline on 2026-10-17: the running tunnel survived until the
router's nightly IP change, then the unit could not restart. The unit now
uses OpenSSH whenever `/usr/bin/ssh` and `tunnel_key.openssh` exist and
falls back to `dbclient` otherwise. OpenSSH cannot read Dropbear keys, so
convert the key once on the board (same key and fingerprint, no server
change; `deploy_kds.sh --setup` does this automatically):

```sh
python3 /usr/local/bin/convert-tunnel-key.py     # → tunnel_key.openssh
ssh-keygen -y -f /etc/rusterando-printer/tunnel_key.openssh | ssh-keygen -lf -
systemctl restart rusterando-tunnel
```

autossh adds nothing here: systemd `Restart=` plus the 15 s keepalive
already reconnect. The board's SSH server is then OpenSSH too, with new host
keys; refresh the laptop's `known_hosts` entry for `davidspizzeria-kds`.

## Rollout next to the Pi: display only

Two printer clients on one shop channel would **both print every
receipt** (each connection gets the live broadcast) and split the acks.
Until the printer moves, run the board as a pure display:

```dotenv
PRINTER_PATH=none                       # never prints, never acks
PRINTER_SHOP_SLUG=davidspizzeria-kds    # own row in the admin Pi panel
```

The header then reads "Nur Anzeige"; the Pi keeps printing as before.
Board moves (Starten / Fertig / Abgeholt / Fahrer unterwegs) work in this
mode — they're the same status changes as the web kitchen board.

## Cutover: printer to the board

1. Pi: `sudo systemctl disable --now rusterando-printer rusterando-tunnel`
2. Move the Epson's USB cable to the board.
3. Board `printer.env`: `PRINTER_PATH=usb`,
   `PRINTER_SHOP_SLUG=davidspizzeria`; `tunnel.env`:
   `TUNNEL_ADMIN_PORT=2222` (and adjust `permitlisten` on the server).
4. `systemctl restart rusterando-tunnel rusterando-kds`

Rollback is the reverse; the Pi's outbox position is untouched (acks are
server-side).

The display survives a printer that is off or out of paper: receipts
wait in a queue (header shows "Drucker aus (n)") and are retried every
5 s, while the board keeps updating.
