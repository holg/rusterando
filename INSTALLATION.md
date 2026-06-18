# Installing Rusterando

End-to-end install guide for someone deploying the server and
(optionally) a kitchen receipt printer from a published release.

> **Quick links**
> - Releases: <https://github.com/holg/rusterando/releases>
> - Source: <https://github.com/holg/rusterando>
> - Kitchen-printer architecture: [docs/kitchen-printer.md](docs/kitchen-printer.md)

## What you're installing

A release tag publishes four tarballs:

| Artifact | What's inside | Where it runs |
|---|---|---|
| `rusterando-server-x86_64-unknown-linux-gnu.tar.gz` | Server binary + `site/` (WASM/JS/CSS) | x86_64 Linux VPS |
| `rusterando-server-aarch64-unknown-linux-gnu.tar.gz` | Same, ARM build | Raspberry Pi 4/5, Ampere/Graviton VPS, ARM Linux |
| `rusterando-printer-x86_64-unknown-linux-gnu.tar.gz` | Kitchen-printer client + systemd unit | x86_64 Linux box wired to an ESC/POS USB printer |
| `rusterando-printer-aarch64-unknown-linux-gnu.tar.gz` | Same, ARM build | Raspberry Pi |

The server is the public website + admin + API. The printer is an
optional companion daemon that prints kitchen receipts when orders
land — only needed if you have an ESC/POS receipt printer on the
LAN (usually plugged via USB into a Pi, but any Linux box with
`/dev/usb/lp0` works).

## 1. Server install

### 1.1 Pick a host

Minimum spec: 1 GB RAM, 5 GB disk, Linux 5.x or newer. A €4/month
VPS comfortably handles a single-shop deployment with a few hundred
orders/day.

### 1.2 System prerequisites

```bash
sudo apt-get update
sudo apt-get install -y libssl3 ca-certificates sqlite3
```

Pick the right tarball for your host's CPU:

```bash
# On the target host:
uname -m      # → x86_64 OR aarch64
```

### 1.3 Lay out the install directory

```bash
sudo install -d -o www-data -g www-data /var/www/rusterando
sudo install -d -o www-data -g www-data /var/www/rusterando/data
cd /var/www/rusterando
```

### 1.4 Unpack the release

Replace `<arch>` with `x86_64-unknown-linux-gnu` or
`aarch64-unknown-linux-gnu`.

```bash
TAG=v0.1.0        # whatever release you're installing
curl -L -o /tmp/server.tar.gz \
    https://github.com/holg/rusterando/releases/download/$TAG/rusterando-server-<arch>.tar.gz
tar -xzf /tmp/server.tar.gz --strip-components=1 -C /tmp/server-extract
sudo cp -R /tmp/server-extract/* /var/www/rusterando/
sudo chown -R www-data:www-data /var/www/rusterando
```

You should now have:

```
/var/www/rusterando/
├── rusterando-server    # the binary
├── site/                # bundled WASM/JS/CSS — served by the binary
├── hash.txt             # cache-busting hash file, consumed at boot
├── data/                # empty; SQLite DB will be created here on first run
└── INSTALLATION.md      # this file (bundled into the tarball)
```

### 1.5 Configure via `.env`

Copy the example from the repo and edit:

```bash
sudo -u www-data tee /var/www/rusterando/.env >/dev/null <<'EOF'
# Where the bound HTTP listener lives. Put `nginx` in front and
# terminate TLS there; this stays loopback-bound.
LEPTOS_SITE_ADDR=127.0.0.1:3001
LEPTOS_OUTPUT_NAME=rusterando

# SQLite database path. Relative to the working directory (which
# the systemd unit pins to /var/www/rusterando).
DATABASE_URL=sqlite:./data/rusterando.db

# Public URL the shop is reachable at. Used for QR codes on receipts
# and for links inside confirmation emails.
PUBLIC_URL=https://yourshop.example.com

# Admin login password. Change this — anyone who knows it can read
# every order, refund payments, etc.
ADMIN_PASSWORD=change-me-now

# Optional: SMTP for order confirmation emails.
SMTP_HOST=smtp.example.com
SMTP_PORT=587
SMTP_USER=...
SMTP_PASS=...
EMAIL_FROM="Your Shop <orders@yourshop.example.com>"

# Optional: Stripe payment integration. Without these, card payment
# is disabled and cash-on-delivery / pay-at-pickup is the only flow.
# STRIPE_SECRET_KEY=sk_live_...
# STRIPE_PUBLISHABLE_KEY=pk_live_...
# STRIPE_WEBHOOK_SECRET=whsec_...

# Optional: kitchen-printer subsystem. See section 2 below.
# KITCHEN_LISTEN_ADDR=127.0.0.1:9001
EOF
sudo chmod 600 /var/www/rusterando/.env
```

### 1.6 systemd unit

```bash
sudo tee /etc/systemd/system/rusterando.service >/dev/null <<'EOF'
[Unit]
Description=Rusterando web server
After=network.target

[Service]
Type=simple
User=www-data
Group=www-data
WorkingDirectory=/var/www/rusterando
ExecStart=/var/www/rusterando/rusterando-server
EnvironmentFile=/var/www/rusterando/.env
Restart=always
RestartSec=5

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/www/rusterando/data

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now rusterando.service
sudo systemctl status rusterando.service --no-pager
```

The server runs migrations on first start; the SQLite database
appears at `/var/www/rusterando/data/rusterando.db`.

### 1.7 nginx + TLS

Drop a vhost in front. Minimum config:

```nginx
server {
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name yourshop.example.com;

    # Let's Encrypt managed via certbot — substitute your own paths.
    ssl_certificate     /etc/letsencrypt/live/yourshop.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/yourshop.example.com/privkey.pem;

    # Static site root (the rsync target — LEPTOS_SITE_ROOT). Used to serve
    # the offline fallback when the app is briefly down during a restart.
    root /var/www/yourshop.example.com/html;

    # Hashed bundles (pkg/<name>.<hash>.{js,wasm,css}). The hash IS the
    # cache key, so these are safe to cache forever — a rebuild changes the
    # filename, never the contents at a URL. immutable = never revalidate.
    location /pkg/ {
        alias /var/www/yourshop.example.com/html/pkg/;
        expires 1y;
        add_header Cache-Control "public, immutable";
    }

    location / {
        proxy_pass http://127.0.0.1:3001;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # CRITICAL: never let the browser cache the HTML document. The page
        # embeds the CURRENT bundle hash in its <script> import; a stale
        # cached page points at a deleted hash after a deploy and the WASM
        # module 404s ("Importing a module script failed"), leaving every
        # button dead. `no-cache` forces revalidation every load so returning
        # customers always get HTML matching the bundles on disk. (The /pkg/
        # block above is content-hashed + immutable, so only the document
        # needs this — and the cost is just the small HTML re-fetch.)
        add_header Cache-Control "no-cache" always;

        # When the upstream is unreachable (the ~5 s restart window) or 5xx,
        # serve the static offline page instead of a raw 502. The app writes
        # `menu.pdf` + `menu-kompakt.pdf` into the site root at boot and after
        # every menu/extras/branding edit, so the fallback page can link to a
        # current menu even while the app is down.
        proxy_intercept_errors on;
        error_page 502 503 504 = @offline;
    }

    # The offline page + the cached menu PDFs are plain static files served
    # straight from disk, so they work precisely when the app does not.
    location @offline {
        rewrite ^ /offline.html break;
    }
    location = /offline.html { }
    location = /menu.pdf { }
    location = /menu-kompakt.pdf { }
}

server {
    listen 80;
    listen [::]:80;
    server_name yourshop.example.com;
    return 301 https://$host$request_uri;
}
```

> The `menu.pdf` / `menu-kompakt.pdf` cache files are (re)written by the app
> into `LEPTOS_SITE_ROOT` at boot and after each menu/extras/branding edit, so
> they survive an `rsync --delete` deploy only if the app has run once since;
> the offline page degrades gracefully (still loads) if a PDF isn't there yet.

### 1.8 First admin login

Visit `https://yourshop.example.com/admin/login`, sign in with
`ADMIN_PASSWORD` from the `.env`. Set up:

- `/admin/branding` — shop name, address, phone, tax id, bank
- `/admin/menu` — categories + menu items
- `/admin/home` — homepage hero / offers / gallery
- `/admin/settings` — theme + free-delivery threshold

## 2. Kitchen printer install (optional)

Skip this section if you don't have an ESC/POS receipt printer.

### 2.1 Wire up the printer

Plug an Epson TM-T20III (or any ESC/POS-compatible USB printer)
into the host that'll drive it. Confirm:

```bash
ls -l /dev/usb/lp0
# crw-rw---- 1 root lp 180, 0 ... /dev/usb/lp0
```

If `/dev/usb/lp0` is missing: the kernel's `usblp` driver should
auto-bind. Check `dmesg | tail` after plugging in. If CUPS is
running it'll steal the device — disable it:

```bash
sudo systemctl disable --now cups cups-browsed
```

### 2.2 SSH reverse tunnel from the printer host to the server

The server listens on `127.0.0.1:9001` (set
`KITCHEN_LISTEN_ADDR=127.0.0.1:9001` in the server's `.env`). The
printer host opens a reverse tunnel so its own `localhost:9001`
maps to the server's `localhost:9001`. The simplest version, with
autossh:

```bash
sudo apt-get install -y autossh
ssh-keygen -t ed25519 -f /etc/rusterando-printer/tunnel_key -N ''
# Append the new pubkey to the server's
# /root/.ssh/authorized_keys (or a `permitopen`-restricted
# dedicated tunnel user — see docs/kitchen-printer.md for the
# secure variant).
```

Then a systemd unit for the tunnel:

```ini
[Unit]
Description=Reverse SSH tunnel to rusterando server
After=network-online.target

[Service]
ExecStart=/usr/bin/autossh -M 0 -N \
    -o ServerAliveInterval=30 -o ExitOnForwardFailure=yes \
    -i /etc/rusterando-printer/tunnel_key \
    -L 127.0.0.1:9001:127.0.0.1:9001 \
    user@yourshop.example.com
Restart=always

[Install]
WantedBy=multi-user.target
```

### 2.3 Install the printer binary + service

```bash
TAG=v0.1.0
ARCH=aarch64-unknown-linux-gnu    # or x86_64-…
curl -L -o /tmp/printer.tar.gz \
    https://github.com/holg/rusterando/releases/download/$TAG/rusterando-printer-$ARCH.tar.gz
tar -xzf /tmp/printer.tar.gz -C /tmp
sudo install -m 755 /tmp/rusterando-printer-$ARCH/rusterando-printer /usr/local/bin/
sudo install -m 644 /tmp/rusterando-printer-$ARCH/systemd/rusterando-printer.service \
    /etc/systemd/system/

sudo install -d -o root -g lp /etc/rusterando-printer
sudo tee /etc/rusterando-printer/printer.env >/dev/null <<'EOF'
PRINTER_SERVER_ADDR=127.0.0.1:9001
PRINTER_SHOP_SLUG=yourshop
PRINTER_PATH=/dev/usb/lp0
PRINTER_STATE_DIR=/var/lib/rusterando-printer
RUST_LOG=rusterando_printer=info
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now rusterando-printer.service
sudo systemctl status rusterando-printer.service --no-pager
```

### 2.4 Smoke test

Place a test order via the website. The kitchen printer should
spit out a receipt within a second.

If nothing happens:
1. Tail the printer journal: `sudo journalctl -u rusterando-printer.service -f`.
2. Tail the server journal on the other end: `sudo journalctl -u rusterando.service -f | grep kitchen`.
3. Verify the SSH tunnel is up: `nc -zv 127.0.0.1 9001` on the printer host.

Full troubleshooting checklist in
[docs/kitchen-printer.md](docs/kitchen-printer.md).

## 3. Upgrading

Releases are forward-compatible at the data level (SQLite migrations
run on boot). The wire protocol between server and printer carries a
[2-byte schema-version envelope](crates/kitchen-protocol/src/lib.rs)
so mismatched versions log + skip rather than crash-loop.

To upgrade:

1. Download the new tarball into a staging dir.
2. `systemctl stop rusterando.service` (and `rusterando-printer.service` if installed).
3. Replace `rusterando-server` + `site/` + `hash.txt` from the new tarball.
4. `systemctl start rusterando.service`.
5. For the printer host: same `install -m 755` step + `systemctl restart`.

Server and printer can be upgraded in either order — the schema
envelope handles the cross-version edge cases.

## 4. Backup

Two files are precious:

| Path | What |
|---|---|
| `/var/www/rusterando/data/rusterando.db` | SQLite database (orders, menu, settings, etc.) |
| `/var/www/rusterando/.env` | Configuration including admin password + Stripe secret + SMTP creds |

A nightly cron job that `sqlite3 .db ".backup …"` and rsyncs the
result off-host is sufficient for a single-shop install. Keep at
least 7 days of rolling backups; the SQLite DB is small.

## 5. License

AGPL-3.0. Run it on your own shop, contribute back if you patch
something useful.
