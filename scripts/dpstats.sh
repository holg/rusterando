#!/usr/bin/env bash
# dpstats.sh — one-shot health dashboard for a deployed davidspizzeria
# / rusterando server. Read-only. Safe to alias to a single keystroke.
#
# Why this exists: 2026-05-30 the prod server quietly accumulated
# CLOSE_WAIT sockets until it hit LimitNOFILE=1024 around 16:00 UTC
# (shop opening time). The 502s rolled in, David was at the shop on
# an iPhone, and the only fix he could reach (button-wise) was the
# /admin "Closed / Force-Open" toggles — which did nothing because
# the server was still alive, just not accepting connections.
#
# The actual fix was `sudo systemctl restart davidspizzeria-server`
# from any SSH session, but you had to KNOW that. Going forward:
# tonight's deploy ships an in-process self-monitor that exits(1) at
# 95% fd usage so systemd's Restart=always picks up before customers
# notice. This script is the second line of defence: a single
# command you can type after `ssh iesna.eu` that shows whether the
# server is healthy at a glance, and if not, what's wrong.
#
# Usage:
#   sudo ./dpstats.sh                  # davids (default)
#   sudo ./dpstats.sh davidspizzeria   # explicit
#   sudo ./dpstats.sh rusterando       # second deployment
#
# Designed for narrow terminals (iPhone SSH client = ~60-80 cols).
# All labels left-aligned to a fixed column so values scan vertically.

set -u

# Resolve which deployment to inspect. Both prod boxes name their
# unit `<app>-server` and live under `/var/www/<app>.de`.
APP="${1:-davidspizzeria}"
SERVICE="${APP}-server"
ROOT="/var/www/${APP}.de"
DB="${ROOT}/data/${APP}.db"

# ANSI colors — only if stdout is a tty (so piping to a file stays
# plain text). iPhone Termius/Blink both honour these.
if [ -t 1 ]; then
  C_OK=$'\033[32m'    # green
  C_WARN=$'\033[33m'  # yellow
  C_ERR=$'\033[31m'   # red
  C_DIM=$'\033[2m'
  C_BOLD=$'\033[1m'
  C_RESET=$'\033[0m'
else
  C_OK=''; C_WARN=''; C_ERR=''; C_DIM=''; C_BOLD=''; C_RESET=''
fi

# Pretty key: value with a fixed-width label so columns align even on
# very narrow terminals. Width chosen so the longest label
# ("kitchen listener:" at 17) fits without truncating values.
LABEL_W=18
row() {
  printf "  %-${LABEL_W}s %s\n" "$1" "$2"
}
hr() {
  printf "${C_DIM}  ─────────────────────────────────────────${C_RESET}\n"
}

# ----- service status -----
echo
printf "${C_BOLD}▌ %s${C_RESET}\n" "$SERVICE"
hr

ACTIVE=$(systemctl is-active "$SERVICE" 2>/dev/null || echo "unknown")
case "$ACTIVE" in
  active)        STATUS_COLOR=$C_OK ;;
  activating)    STATUS_COLOR=$C_WARN ;;
  failed|inactive|*) STATUS_COLOR=$C_ERR ;;
esac
row "service:" "${STATUS_COLOR}${ACTIVE}${C_RESET}"

# Process info — systemctl show is the scriptable variant, but it
# returns key=value lines so we don't need to parse human-readable
# `status` output.
PID=$(systemctl show "$SERVICE" --property=MainPID --value 2>/dev/null)
SINCE=$(systemctl show "$SERVICE" --property=ActiveEnterTimestamp --value 2>/dev/null)
RESTARTS=$(systemctl show "$SERVICE" --property=NRestarts --value 2>/dev/null)
row "pid:" "${PID:-?}"
row "running since:" "${SINCE:-?}"

# Uptime in human terms — many seconds turns into "Xh Ym".
if [ -n "${PID:-}" ] && [ "$PID" != "0" ] && [ -d "/proc/$PID" ]; then
  # /proc/<pid>/stat field 22 is starttime in clock ticks since boot;
  # easier path: use `ps -o etime=` which formats as DD-HH:MM:SS.
  UPTIME=$(ps -o etime= -p "$PID" 2>/dev/null | tr -d ' ' || echo "?")
  row "uptime:" "${UPTIME}"
fi
row "restarts:" "${RESTARTS:-0}"

# ----- file descriptors (the yesterday-killer) -----
echo
printf "${C_BOLD}▌ fds${C_RESET}\n"
hr

if [ -n "${PID:-}" ] && [ "$PID" != "0" ] && [ -d "/proc/$PID/fd" ]; then
  FDS=$(ls "/proc/$PID/fd" 2>/dev/null | wc -l)
  # The soft limit straight from /proc/<pid>/limits — wide-table
  # output, "Max open files" line, second numeric column is the soft
  # cap (`rlim_cur`). awk slices it without bash arrays.
  FD_LIMIT=$(awk '/Max open files/ { print $4 }' "/proc/$PID/limits" 2>/dev/null)
  if [ -n "$FD_LIMIT" ] && [ "$FD_LIMIT" -gt 0 ]; then
    FD_PCT=$(( FDS * 100 / FD_LIMIT ))
  else
    FD_PCT=0
  fi
  # Same thresholds the in-process self-monitor uses (health.rs):
  # < 50 ok, 50-79 warn, 80+ crit.
  if [ "$FD_PCT" -ge 80 ]; then FDC=$C_ERR
  elif [ "$FD_PCT" -ge 50 ]; then FDC=$C_WARN
  else FDC=$C_OK
  fi
  row "open:" "${FDC}${FDS}${C_RESET} / ${FD_LIMIT}  (${FDC}${FD_PCT}%${C_RESET})"

  # CLOSE_WAIT is yesterday's specific symptom — call it out so a
  # silent leak shows up here before it triggers the % threshold.
  CW=$(ss -tn state close-wait 2>/dev/null | wc -l)
  # ss -tn includes a header row when it finds anything; subtract.
  if [ "$CW" -gt 1 ]; then CW=$((CW - 1)); else CW=0; fi
  if [ "$CW" -ge 20 ]; then CWC=$C_ERR
  elif [ "$CW" -ge 5 ]; then CWC=$C_WARN
  else CWC=$C_OK
  fi
  row "CLOSE_WAIT:" "${CWC}${CW}${C_RESET}"
else
  row "open:" "${C_DIM}(pid not found)${C_RESET}"
fi

# ----- memory -----
echo
printf "${C_BOLD}▌ memory${C_RESET}\n"
hr

if [ -n "${PID:-}" ] && [ -r "/proc/$PID/status" ]; then
  # VmRSS = resident set size in KB.
  RSS_KB=$(awk '/^VmRSS:/ { print $2 }' "/proc/$PID/status" 2>/dev/null)
  if [ -n "$RSS_KB" ]; then
    RSS_MB=$(( RSS_KB / 1024 ))
    row "rss:" "${RSS_MB} MB"
  fi
  # Tasks (= threads) — tokio worker count + a few. Spike = leaked
  # spawn loop.
  THREADS=$(awk '/^Threads:/ { print $2 }' "/proc/$PID/status" 2>/dev/null)
  row "threads:" "${THREADS:-?}"
fi

# Whole-box free memory — if the box is hot but the process is fine,
# the bottleneck is elsewhere (nginx, another shop's deployment).
if [ -r /proc/meminfo ]; then
  TOTAL_KB=$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)
  AVAIL_KB=$(awk '/^MemAvailable:/ { print $2 }' /proc/meminfo)
  if [ -n "$TOTAL_KB" ] && [ -n "$AVAIL_KB" ] && [ "$TOTAL_KB" -gt 0 ]; then
    USED_PCT=$(( (TOTAL_KB - AVAIL_KB) * 100 / TOTAL_KB ))
    if [ "$USED_PCT" -ge 90 ]; then MC=$C_ERR
    elif [ "$USED_PCT" -ge 75 ]; then MC=$C_WARN
    else MC=$C_OK
    fi
    AVAIL_MB=$(( AVAIL_KB / 1024 ))
    row "host free:" "${AVAIL_MB} MB  (${MC}${USED_PCT}% used${C_RESET})"
  fi
fi

# ----- database -----
echo
printf "${C_BOLD}▌ database${C_RESET}\n"
hr

if [ -r "$DB" ]; then
  DB_SIZE=$(du -h "$DB" 2>/dev/null | awk '{print $1}')
  row "$(basename "$DB"):" "${DB_SIZE}"
  # WAL + shm files exist while sqlite has open writers; size tells
  # you if a checkpoint is overdue (normally < 1 MB).
  if [ -r "${DB}-wal" ]; then
    WAL_SIZE=$(du -h "${DB}-wal" 2>/dev/null | awk '{print $1}')
    row "  wal:" "${WAL_SIZE}"
  fi
  # Order count + most-recent timestamp answer "is the shop actively
  # taking orders right now?" without poking the admin UI.
  if command -v sqlite3 >/dev/null 2>&1; then
    COUNT=$(sqlite3 "$DB" "SELECT COUNT(*) FROM orders WHERE created_at > datetime('now', '-24 hours');" 2>/dev/null || echo "?")
    row "orders 24h:" "${COUNT}"
    LATEST=$(sqlite3 "$DB" "SELECT order_number, created_at FROM orders ORDER BY created_at DESC LIMIT 1;" 2>/dev/null | tr '|' ' ' || echo "")
    if [ -n "$LATEST" ]; then
      row "latest:" "${LATEST}"
    fi
  fi
fi

# ----- disk -----
echo
printf "${C_BOLD}▌ disk${C_RESET}\n"
hr

# /var/www is where the binary + DB + uploads live. Anything else
# (logs, packages) is on /. Show both.
DF_OUT=$(df -h --output=avail,pcent "$ROOT" 2>/dev/null | tail -1)
if [ -n "$DF_OUT" ]; then
  AVAIL=$(echo "$DF_OUT" | awk '{print $1}')
  PCT=$(echo "$DF_OUT" | awk '{print $2}' | tr -d '%')
  if [ "${PCT:-0}" -ge 90 ]; then DC=$C_ERR
  elif [ "${PCT:-0}" -ge 75 ]; then DC=$C_WARN
  else DC=$C_OK
  fi
  row "$ROOT:" "${AVAIL} free  (${DC}${PCT}% used${C_RESET})"
fi

# ----- kitchen -----
echo
printf "${C_BOLD}▌ kitchen${C_RESET}\n"
hr

# Is the kitchen channel currently CONNECTED? Long-lived TCP — once
# the Pi connects, it stays connected and reconnect/hello log lines
# only appear on a drop. So "connected" = an ESTABLISHED inbound to
# port 9001 from a non-server peer right now, NOT a recent log line.
# (Earlier draft greped journal for hello+connected and reported a
# false "no activity in 5 min" whenever the Pi sat quietly chatting
# on an idle socket — exactly the healthy state we don't want to
# flag.)
KITCHEN_CONNS=$(ss -tn state established '( sport = :9001 )' 2>/dev/null \
  | awk 'NR>1' | wc -l)
if [ "$KITCHEN_CONNS" -ge 1 ]; then
  row "channel:" "${C_OK}connected (${KITCHEN_CONNS} client$([ "$KITCHEN_CONNS" -gt 1 ] && echo s))${C_RESET}"
else
  row "channel:" "${C_WARN}no client connected${C_RESET}"
fi
# Most recent kitchen log line (any kind) — gives the last activity
# timestamp without depending on it for the health verdict.
LAST_KITCHEN=$(journalctl -u "$SERVICE" --since '24 hours ago' --no-pager 2>/dev/null \
  | grep -iE 'kitchen' \
  | tail -1 \
  | sed -E 's/^([A-Z][a-z]+ +[0-9]+ [0-9:]+).*kitchen[^:]*: */\1 /' \
  | cut -c1-60)
if [ -n "$LAST_KITCHEN" ]; then
  row "last log:" "${C_DIM}${LAST_KITCHEN}${C_RESET}"
fi

# ----- recent errors -----
echo
printf "${C_BOLD}▌ recent ERROR / WARN (last 1h)${C_RESET}\n"
hr

# Filter on systemd's own priority field rather than greping for the
# word "error" so noisy customer errors (e.g. typst warnings during
# pdf render) don't drown out the real signal. journalctl prints
# "-- No entries --" when nothing matches; treat that literal string
# as empty too.
ERR_OUT=$(journalctl -u "$SERVICE" --since '1 hour ago' --no-pager -p warning 2>/dev/null \
  | grep -vE 'typst warn:|^-- No entries --' \
  | tail -10)
if [ -n "$ERR_OUT" ]; then
  # Print at most 10 lines, dim-colored so they don't dominate, with
  # a 2-space indent matching the rest of the layout.
  echo "$ERR_OUT" | sed "s/^/  ${C_DIM}/" | sed "s/$/${C_RESET}/"
else
  row "" "${C_OK}clean${C_RESET}"
fi

echo
