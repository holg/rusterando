#!/bin/bash
# Rusterando deploy script.
#
# Each deployment has its own .env file at the repo root. By default
# the script reads `.env`. Pass `-e <name>` to load `.env.<name>`
# instead — used to run multiple deployments off the same checkout
# (e.g. davidspizzeria.de + rusterando.de on the same VPS, different
# ports).
#
# Required `.env*` keys for a deploy:
#   APP_NAME             = name of the binary on disk + systemd unit
#                          (e.g. davidspizzeria-server, default rusterando-server)
#   BIN_NAME             = same as APP_NAME (kept as alias for build-time use;
#                          cargo produces target/<profile>/rusterando-server,
#                          this script renames it to APP_NAME during rsync)
#   LEPTOS_OUTPUT_NAME   = JS bundle name in /pkg/<name>.<hash>.js
#                          (e.g. davidspizzeria, default rusterando)
#   LEPTOS_SITE_ADDR     = bind address, e.g. 127.0.0.1:3001 / 127.0.0.1:3002
#                          (each deployment binds a different localhost port;
#                          nginx proxy_pass routes its vhost to the right one)
#   DEPLOY_REMOTE_BASE   = e.g. /var/www/davidspizzeria.de
#   SSH_HOST             = e.g. iesna.eu
#
# Run from your local machine.

set -e

# =============================================================================
# Parse `-e <name>` (env profile) before anything else, then load the
# matching .env file. Defaults to `.env` for back-compat.
# =============================================================================
ENV_PROFILE=""
if [[ "${1:-}" == "-e" || "${1:-}" == "--env" ]]; then
    if [[ -z "${2:-}" ]]; then
        echo "✗ -e requires a profile name (e.g. -e davids)" >&2
        exit 1
    fi
    ENV_PROFILE="$2"
    shift 2
fi

ENV_FILE=".env"
[[ -n "$ENV_PROFILE" ]] && ENV_FILE=".env.$ENV_PROFILE"

if [ -f "$ENV_FILE" ]; then
    set -a
    # shellcheck disable=SC1090
    . "$ENV_FILE"
    set +a
elif [[ -n "$ENV_PROFILE" ]]; then
    echo "✗ env profile '$ENV_PROFILE' selected but $ENV_FILE not found" >&2
    exit 1
fi
echo "Using env file: $ENV_FILE"

# =============================================================================
# Configuration (with sensible upstream defaults for fresh checkouts)
# =============================================================================
SSH_HOST="${SSH_HOST:?SSH_HOST not set in .env}"
APP_NAME="${APP_NAME:-${BIN_NAME:-rusterando-server}}"
REMOTE_BASE="${DEPLOY_REMOTE_BASE:-${REMOTE_BASE:-/var/www/example.com}}"
TARGET_TRIPLE="${TARGET_TRIPLE:-x86_64-unknown-linux-gnu}"

# Cargo's [[bin]] is hard-coded to "rusterando-server" in upstream; the
# build always produces this filename. We rename to $APP_NAME during
# upload so per-deployment systemd units don't have to change.
CARGO_BIN_NAME="rusterando-server"

# All deploys build with the `release-prod` profile (fat LTO,
# codegen-units=1). Wired through workspace metadata so cargo-leptos's
# wasm-bindgen step looks at the same dir cargo wrote into.
BUILD_PROFILE="release-prod"

# Local paths
LOCAL_BUILD_DIR="target/$TARGET_TRIPLE/$BUILD_PROFILE"
LOCAL_SITE_DIR="target/site"

# Remote paths
REMOTE_BIN_DIR="$REMOTE_BASE"
REMOTE_HTML_DIR="$REMOTE_BASE/html"
REMOTE_DATA_DIR="$REMOTE_BASE/data"
REMOTE_BACKUP_DIR="$REMOTE_BASE/backups"

# =============================================================================
# Usage
# =============================================================================
usage() {
    cat <<EOF
Usage: $0 [-e <env-profile>] <command>

Options:
    -e, --env <name>   Load .env.<name> instead of .env. Used to run
                       multiple deployments from the same checkout
                       (e.g. -e davids vs -e demo on the same VPS).

Commands:
    build         Build the release binary for Linux (x86)
    deploy        Backup + Upload + Restart + Apply seeds
    full          Build + Backup + Upload + Restart + Apply seeds
    upload        Upload built files to server
    setup         First-time setup: create dirs, install systemd service
    restart       Restart the service on server
    apply-seeds   Apply per-deployment seed SQL (data/*.deploy.sql,
                  data/*.<env-profile>.sql) — idempotent, safe to re-run
    logs          Show service logs (follow)
    status        Show service status
    backup        Create a backup of the current version on the server
    backups       List all available backups
    restore       Restore a backup (interactive selection)

Configuration is read from .env (or .env.<profile> with -e). Required
keys: APP_NAME, BIN_NAME, LEPTOS_OUTPUT_NAME, LEPTOS_SITE_ADDR,
DEPLOY_REMOTE_BASE, SSH_HOST. See README + .env.example.

Examples:
    $0 full                       # Default deployment (.env)
    $0 -e davids full             # Davids deployment (.env.davids)
    $0 -e demo full               # Demo deployment (.env.demo, port 3002)
    $0 -e davids backup           # Manual backup of Davids
    $0 -e davids restore          # Restore a Davids backup
    $0 restore                   # Restore a previous version
    $0 logs                      # Tail logs
EOF
    exit 1
}

# =============================================================================
# Helpers
# =============================================================================
ssh_cmd() {
    ssh "$SSH_HOST" "$@"
}

# =============================================================================
# Commands
# =============================================================================
cmd_build() {
    echo "=== Building for $TARGET_TRIPLE (profile: $BUILD_PROFILE) ==="
    echo "    cargo bin name: $CARGO_BIN_NAME"
    echo "    deployed as:    $APP_NAME"

    # AASA overlay: if a per-shop variant exists (e.g. .davids), copy
    # it over the placeholder before the build picks it up via
    # include_bytes! in main.rs. The placeholder file is committed to
    # git; the overlay is gitignored. After the build we restore the
    # placeholder so the working tree stays clean.
    AASA="public/.well-known/apple-app-site-association"
    AASA_OVERLAY="${AASA_OVERLAY:-${AASA}.davids}"
    AASA_RESTORED=0
    if [[ -f "$AASA_OVERLAY" && -f "$AASA" ]]; then
        cp "$AASA" "${AASA}.placeholder.tmp"
        cp "$AASA_OVERLAY" "$AASA"
        AASA_RESTORED=1
        echo "    AASA overlay applied from $AASA_OVERLAY"
    fi
    # Always restore on exit so a failed build doesn't leave the real
    # values sitting in the public-tracked file.
    trap 'if [[ "$AASA_RESTORED" == "1" && -f "${AASA}.placeholder.tmp" ]]; then mv "${AASA}.placeholder.tmp" "$AASA"; fi' EXIT

    ./scripts/cross_build_on_mac.sh "$TARGET_TRIPLE"

    if [[ -f "$LOCAL_BUILD_DIR/$CARGO_BIN_NAME" ]]; then
        echo "✓ Binary built: $LOCAL_BUILD_DIR/$CARGO_BIN_NAME"
        echo "  Size: $(du -h "$LOCAL_BUILD_DIR/$CARGO_BIN_NAME" | cut -f1)"
    else
        echo "✗ Build failed - binary not found at $LOCAL_BUILD_DIR/$CARGO_BIN_NAME"
        exit 1
    fi
}

cmd_backup() {
    echo "=== Creating backup on $SSH_HOST ==="

    # Backup file prefix is derived from APP_NAME so each deployment's
    # backups are namespaced (davidspizzeria_*.zip on Davids' server,
    # rusterando_*.zip elsewhere).
    BACKUP_PREFIX="${APP_NAME%-server}"

    # Unquoted heredoc so the four locals expand here; everything that
    # must evaluate on the remote uses \$.
    ssh_cmd "sudo bash -s" <<REMOTE_EOF
set -e

REMOTE_BASE="$REMOTE_BASE"
APP_NAME="$APP_NAME"
BACKUP_PREFIX="$BACKUP_PREFIX"
BACKUP_DIR="\$REMOTE_BASE/backups"

mkdir -p "\$BACKUP_DIR"

if [ ! -f "\$REMOTE_BASE/\$APP_NAME" ]; then
    echo "No binary found at \$REMOTE_BASE/\$APP_NAME - nothing to back up"
    exit 0
fi

TIMESTAMP=\$(date +%Y%m%d_%H%M%S)
BACKUP_NAME="\${BACKUP_PREFIX}_\${TIMESTAMP}"
STAGING="/tmp/\$BACKUP_NAME"

mkdir -p "\$STAGING"
cp "\$REMOTE_BASE/\$APP_NAME" "\$STAGING/"
[ -f "\$REMOTE_BASE/hash.txt" ] && cp "\$REMOTE_BASE/hash.txt" "\$STAGING/"
[ -f "\$REMOTE_BASE/.env" ] && cp "\$REMOTE_BASE/.env" "\$STAGING/"
[ -d "\$REMOTE_BASE/html" ] && cp -r "\$REMOTE_BASE/html" "\$STAGING/"
[ -d "\$REMOTE_BASE/data" ] && cp -r "\$REMOTE_BASE/data" "\$STAGING/"
echo "Backup created: \$TIMESTAMP" > "\$STAGING/BACKUP_INFO.txt"

cd /tmp
zip -qr "\$BACKUP_DIR/\${BACKUP_NAME}.zip" "\$BACKUP_NAME"
rm -rf "\$STAGING"

SIZE=\$(du -h "\$BACKUP_DIR/\${BACKUP_NAME}.zip" | cut -f1)
echo "✓ Backup created: \${BACKUP_NAME}.zip (\$SIZE)"

cd "\$BACKUP_DIR"
BACKUP_COUNT=\$(ls -1 \${BACKUP_PREFIX}_*.zip 2>/dev/null | wc -l)
if [ "\$BACKUP_COUNT" -gt 10 ]; then
    REMOVE_COUNT=\$((BACKUP_COUNT - 10))
    ls -1t \${BACKUP_PREFIX}_*.zip | tail -n "\$REMOVE_COUNT" | xargs rm -f
    echo "Cleaned up \$REMOVE_COUNT old backup(s), keeping last 10"
fi
REMOTE_EOF
}

cmd_backups() {
    echo "=== Available backups on $SSH_HOST ==="
    echo ""

    BACKUP_PREFIX="${APP_NAME%-server}"

    ssh_cmd "sudo bash -s" <<REMOTE_EOF
BACKUP_DIR="$REMOTE_BASE/backups"
BACKUP_PREFIX="$BACKUP_PREFIX"

if [ ! -d "\$BACKUP_DIR" ] || [ -z "\$(ls -A "\$BACKUP_DIR"/\${BACKUP_PREFIX}_*.zip 2>/dev/null)" ]; then
    echo "No backups found."
    exit 0
fi

printf "%-4s  %-30s  %s\n" "#" "Backup" "Size"
printf "%-4s  %-30s  %s\n" "---" "------------------------------" "--------"

INDEX=1
for f in \$(ls -1t "\$BACKUP_DIR"/\${BACKUP_PREFIX}_*.zip 2>/dev/null); do
    NAME=\$(basename "\$f" .zip)
    SIZE=\$(du -h "\$f" | cut -f1)
    TS=\$(echo "\$NAME" | sed "s/\${BACKUP_PREFIX}_//")
    DATE=\$(echo "\$TS" | sed 's/\\([0-9]\\{4\\}\\)\\([0-9]\\{2\\}\\)\\([0-9]\\{2\\}\\)_\\([0-9]\\{2\\}\\)\\([0-9]\\{2\\}\\)\\([0-9]\\{2\\}\\)/\\1-\\2-\\3 \\4:\\5:\\6/')
    printf "%-4s  %-30s  %s\n" "\$INDEX" "\$DATE" "\$SIZE"
    INDEX=\$((INDEX + 1))
done
REMOTE_EOF
}

cmd_restore() {
    echo "=== Restore backup on $SSH_HOST ==="
    echo ""

    BACKUP_PREFIX="${APP_NAME%-server}"

    BACKUP_LIST=$(ssh_cmd "sudo ls -1t $REMOTE_BASE/backups/${BACKUP_PREFIX}_*.zip 2>/dev/null" || true)

    if [[ -z "$BACKUP_LIST" ]]; then
        echo "No backups found."
        exit 1
    fi

    echo "Available backups:"
    echo ""
    INDEX=1
    while IFS= read -r filepath; do
        NAME=$(basename "$filepath" .zip)
        TS=$(echo "$NAME" | sed "s/${BACKUP_PREFIX}_//")
        DATE=$(echo "$TS" | sed 's/\([0-9]\{4\}\)\([0-9]\{2\}\)\([0-9]\{2\}\)_\([0-9]\{2\}\)\([0-9]\{2\}\)\([0-9]\{2\}\)/\1-\2-\3 \4:\5:\6/')
        SIZE=$(ssh_cmd "sudo du -h '$filepath'" | cut -f1)
        printf "  %2d) %s  (%s)\n" "$INDEX" "$DATE" "$SIZE"
        INDEX=$((INDEX + 1))
    done <<< "$BACKUP_LIST"

    echo ""
    printf "Select backup to restore (1-%d), or 'q' to cancel: " "$((INDEX - 1))"
    read -r SELECTION

    if [[ "$SELECTION" == "q" || "$SELECTION" == "Q" ]]; then
        echo "Cancelled."
        exit 0
    fi

    if ! [[ "$SELECTION" =~ ^[0-9]+$ ]] || [[ "$SELECTION" -lt 1 ]] || [[ "$SELECTION" -ge "$INDEX" ]]; then
        echo "Invalid selection."
        exit 1
    fi

    SELECTED_FILE=$(echo "$BACKUP_LIST" | sed -n "${SELECTION}p")
    SELECTED_NAME=$(basename "$SELECTED_FILE" .zip)

    echo ""
    echo "WARNING: This will:"
    echo "  - Stop the running service"
    echo "  - Replace: $APP_NAME, hash.txt, html/, .env, data/"
    echo "  - Restart the service"
    echo ""
    echo "Selected: $SELECTED_NAME"
    printf "Are you sure? (yes/no): "
    read -r CONFIRM

    if [[ "$CONFIRM" != "yes" ]]; then
        echo "Cancelled."
        exit 0
    fi

    echo ""
    echo "--- Backing up current version before restore ---"
    cmd_backup

    echo ""
    echo "--- Restoring $SELECTED_NAME ---"

    ssh_cmd "sudo bash -s" <<REMOTE_EOF
set -e

REMOTE_BASE="$REMOTE_BASE"
APP_NAME="$APP_NAME"
BACKUP_FILE="$SELECTED_FILE"
BACKUP_NAME="$SELECTED_NAME"

echo "Stopping service..."
systemctl stop "\$APP_NAME" || true

echo "Extracting backup..."
cd /tmp
rm -rf "\$BACKUP_NAME"
unzip -qo "\$BACKUP_FILE"

# Restore binary
if [ -f "/tmp/\$BACKUP_NAME/\$APP_NAME" ]; then
    cp "/tmp/\$BACKUP_NAME/\$APP_NAME" "\$REMOTE_BASE/"
    chmod +x "\$REMOTE_BASE/\$APP_NAME"
    echo "  Restored: \$APP_NAME"
fi

# Restore hash.txt
if [ -f "/tmp/\$BACKUP_NAME/hash.txt" ]; then
    cp "/tmp/\$BACKUP_NAME/hash.txt" "\$REMOTE_BASE/"
    echo "  Restored: hash.txt"
fi

# Restore .env
if [ -f "/tmp/\$BACKUP_NAME/.env" ]; then
    cp "/tmp/\$BACKUP_NAME/.env" "\$REMOTE_BASE/"
    chmod 600 "\$REMOTE_BASE/.env"
    echo "  Restored: .env"
fi

# Restore html/
if [ -d "/tmp/\$BACKUP_NAME/html" ]; then
    rm -rf "\$REMOTE_BASE/html"
    cp -r "/tmp/\$BACKUP_NAME/html" "\$REMOTE_BASE/"
    echo "  Restored: html/"
fi

# Restore data/
if [ -d "/tmp/\$BACKUP_NAME/data" ]; then
    rm -rf "\$REMOTE_BASE/data"
    cp -r "/tmp/\$BACKUP_NAME/data" "\$REMOTE_BASE/"
    echo "  Restored: data/"
fi

# Fix permissions
chown -R www-data:www-data "\$REMOTE_BASE"

# Clean up
rm -rf "/tmp/\$BACKUP_NAME"

echo "Starting service..."
systemctl start "\$APP_NAME"
sleep 1
systemctl status "\$APP_NAME" --no-pager || true
REMOTE_EOF

    echo ""
    echo "=== Restore complete ==="
    echo "Site: https://davidspizzeria.de"
}

cmd_upload() {
    echo "=== Uploading to $SSH_HOST ==="

    # Create directories on server
    echo "Creating directories..."
    ssh_cmd "sudo mkdir -p $REMOTE_BIN_DIR $REMOTE_HTML_DIR $REMOTE_DATA_DIR $REMOTE_BACKUP_DIR && sudo chown -R \$(whoami) $REMOTE_BASE"

    # Upload binary, renaming on the wire from the upstream cargo bin name
    # ($CARGO_BIN_NAME) to whatever this deployment calls it ($APP_NAME).
    # Davids' .env sets APP_NAME=davidspizzeria-server so the on-server
    # filename + systemd unit don't change across the rusterando rename.
    echo "Uploading binary as $APP_NAME..."
    rsync -avz --progress "$LOCAL_BUILD_DIR/$CARGO_BIN_NAME" "$SSH_HOST:$REMOTE_BIN_DIR/$APP_NAME"
    if [[ -f "$LOCAL_BUILD_DIR/hash.txt" ]]; then
        echo "Uploading hash.txt..."
        rsync -avz --progress "$LOCAL_BUILD_DIR/hash.txt" "$SSH_HOST:$REMOTE_BIN_DIR/"
    fi

    # Upload site assets (WASM, JS, CSS, static files)
    echo "Uploading site assets..."
    rsync -avz --delete "$LOCAL_SITE_DIR/" "$SSH_HOST:$REMOTE_HTML_DIR/"

    # Upload .env: ship the locally-loaded $ENV_FILE as .env on the
    # server (systemd's EnvironmentFile= reads from a fixed path per
    # deployment). On first deploy this seeds the file; on subsequent
    # deploys it overwrites — your local copy is the source of truth.
    if [[ -f "$ENV_FILE" ]]; then
        echo "Uploading $ENV_FILE as $REMOTE_BIN_DIR/.env..."
        scp -q "$ENV_FILE" "$SSH_HOST:$REMOTE_BIN_DIR/.env"
        ssh_cmd "sudo chmod 600 $REMOTE_BIN_DIR/.env"
    fi

    # Fix permissions
    ssh_cmd "sudo chown -R www-data:www-data $REMOTE_BASE && sudo chmod +x $REMOTE_BIN_DIR/$APP_NAME"

    echo "✓ Upload complete"
}

cmd_setup() {
    echo "=== First-time setup on $SSH_HOST ==="

    # Create systemd service. LEPTOS_OUTPUT_NAME comes from the .env
    # file loaded via EnvironmentFile= below — we don't hard-code it
    # here so a single .env edit (LEPTOS_OUTPUT_NAME=foo) is enough for
    # any future rename without rewriting this unit file.
    LEPTOS_OUTPUT_NAME_FOR_UNIT="${LEPTOS_OUTPUT_NAME:-rusterando}"
    cat <<EOF | ssh_cmd "sudo tee /etc/systemd/system/$APP_NAME.service > /dev/null"
[Unit]
Description=$APP_NAME web server
After=network.target

[Service]
Type=simple
User=www-data
Group=www-data
WorkingDirectory=$REMOTE_BIN_DIR
ExecStart=$REMOTE_BIN_DIR/$APP_NAME
Restart=always
RestartSec=5
Environment=RUST_LOG=info
Environment=LEPTOS_HASH_FILES=true
Environment=LEPTOS_SITE_ROOT=$REMOTE_HTML_DIR
Environment=LEPTOS_OUTPUT_NAME=$LEPTOS_OUTPUT_NAME_FOR_UNIT
# LEPTOS_SITE_ADDR comes from the .env file below — every deployment
# binds its own localhost port (e.g. 127.0.0.1:3001 vs 3002) and
# nginx vhost configs route to the right one.
EnvironmentFile=-$REMOTE_BIN_DIR/.env

[Install]
WantedBy=multi-user.target
EOF

    ssh_cmd "sudo bash -s" <<EOF
set -e

echo "Setting permissions..."
chown -R www-data:www-data $REMOTE_BASE
chmod +x $REMOTE_BIN_DIR/$APP_NAME
chmod 600 $REMOTE_BIN_DIR/.env 2>/dev/null || true

echo "Enabling systemd service..."
systemctl daemon-reload
systemctl enable $APP_NAME

echo "Starting service..."
systemctl restart $APP_NAME

echo "=== Setup complete ==="
systemctl status $APP_NAME --no-pager || true
EOF
}

cmd_deploy() {
    # The cargo bin is always named $CARGO_BIN_NAME locally; we rename
    # it to $APP_NAME during upload (per .env).
    if [[ ! -f "$LOCAL_BUILD_DIR/$CARGO_BIN_NAME" ]]; then
        echo "✗ No binary found at $LOCAL_BUILD_DIR/$CARGO_BIN_NAME"
        echo "  Run '$0 build' first!"
        exit 1
    fi
    echo "Using binary: $LOCAL_BUILD_DIR/$CARGO_BIN_NAME → server:$APP_NAME ($(du -h "$LOCAL_BUILD_DIR/$CARGO_BIN_NAME" | cut -f1))"
    cmd_backup
    cmd_upload
    cmd_restart
    cmd_apply_seeds
    echo ""
    echo "=== Deploy complete ==="
    echo "Site: https://davidspizzeria.de"
}

# Apply per-deployment seed SQL files (e.g. branding.davids.sql) against
# the prod SQLite DB after migrations have run. Each *.sql file under
# data/ that ends in `.davids.sql` (or generally `.deploy.sql`) is
# uploaded, executed via sqlite3, then the service is restarted so the
# in-memory caches (BrandingHandle, ThemeHandle) pick up the new rows.
#
# Idempotent: every UPDATE in the seed is guarded with `WHERE value=...`
# clauses that only match the placeholder defaults from migrations, so
# re-running on an already-customised DB is a no-op.
cmd_apply_seeds() {
    # Seed files are matched in two ways:
    #   1. `*.deploy.sql`        — applied to every deployment.
    #   2. `*.<env-profile>.sql` — applied only when -e <env-profile> is in
    #                              effect (e.g. data/branding.davids.sql when
    #                              run with `-e davids`).
    # Falls back to the APP_NAME-derived prefix for back-compat with the
    # pre-`-e` scripts that named seeds after the binary suffix.
    local profile="${ENV_PROFILE:-${APP_NAME%-server}}"
    local seeds=()
    shopt -s nullglob
    for f in data/*.deploy.sql data/*."${profile}".sql; do
        [[ -f "$f" ]] && seeds+=("$f")
    done
    shopt -u nullglob

    if [[ ${#seeds[@]} -eq 0 ]]; then
        return 0
    fi

    echo ""
    echo "=== Applying ${#seeds[@]} per-deployment seed SQL file(s) ==="

    # Resolve the prod SQLite file path.
    #
    # The server's WorkingDirectory is $REMOTE_BASE; sqlx opens the DB
    # by the relative path in $DATABASE_URL ("sqlite:./data/foo.db"), so
    # we strip the prefix and resolve against $REMOTE_BASE here.
    # Falls back to "${profile}.db" if DATABASE_URL isn't set.
    local db_rel="${DATABASE_URL#sqlite:}"
    db_rel="${db_rel#./}"
    if [[ -z "$db_rel" || "$db_rel" == "$DATABASE_URL" ]]; then
        db_rel="data/${profile}.db"
    fi
    local remote_db="$REMOTE_BASE/$db_rel"

    for seed in "${seeds[@]}"; do
        local base
        base="$(basename "$seed")"
        echo "  → uploading $base"
        scp -q "$seed" "$SSH_HOST:/tmp/$base"
        echo "  → applying  $base on $remote_db"
        ssh_cmd "sudo sqlite3 '$remote_db' < '/tmp/$base' && sudo rm -f '/tmp/$base'"
    done

    echo "  → restarting $APP_NAME so in-memory caches pick up the seeds"
    ssh_cmd "sudo systemctl restart $APP_NAME"
}

cmd_full() {
    cmd_build
    cmd_deploy
}

cmd_restart() {
    echo "=== Restarting $APP_NAME on $SSH_HOST ==="
    ssh_cmd "sudo systemctl restart $APP_NAME && sleep 1 && sudo systemctl status $APP_NAME --no-pager"
}

cmd_logs() {
    echo "=== Logs from $SSH_HOST (Ctrl+C to exit) ==="
    ssh_cmd "sudo journalctl -u $APP_NAME -f"
}

cmd_status() {
    echo "=== Status on $SSH_HOST ==="
    ssh_cmd "sudo systemctl status $APP_NAME --no-pager"
    echo ""
    echo "=== Recent logs ==="
    ssh_cmd "sudo journalctl -u $APP_NAME -n 20 --no-pager"
}

# =============================================================================
# Main
# =============================================================================
COMMAND="${1:-}"

case "$COMMAND" in
    build)        cmd_build ;;
    deploy)       cmd_deploy ;;
    full)         cmd_full ;;
    upload)       cmd_upload ;;
    setup)        cmd_setup ;;
    restart)      cmd_restart ;;
    apply-seeds)  cmd_apply_seeds ;;
    logs)         cmd_logs ;;
    status)       cmd_status ;;
    backup)       cmd_backup ;;
    backups)      cmd_backups ;;
    restore)      cmd_restore ;;
    *)            usage ;;
esac
