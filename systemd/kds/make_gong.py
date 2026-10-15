#!/usr/bin/env python3
"""Generate the kitchen display's new-order gong: a two-note "ding-dong"
(E5 → C5) with bell overtones and a slow fade, 0.3 s of silence first so a
Bluetooth speaker waking up doesn't swallow the start. 48 kHz mono 16-bit.

    python3 systemd/kds/make_gong.py crates/rusterando-printer/assets/sounds/gong.wav
"""
import math, struct, sys, wave

SR = 48000
NOTES = [(659.25, 0.30), (523.25, 0.95)]  # (Hz, start s): E5 "ding", C5 "dong"
# Bell partials: (frequency multiple, amplitude, decay per second).
PARTIALS = [(1.0, 1.0, 1.6), (2.0, 0.5, 2.6), (2.76, 0.35, 3.5), (5.4, 0.15, 6.0), (0.5, 0.25, 1.2)]

buf = [0.0] * int(3.0 * SR)
for f, t0 in NOTES:
    start = int(t0 * SR)
    for i in range(int(2.0 * SR)):
        if start + i >= len(buf):
            break
        t = i / SR
        attack = min(1.0, t / 0.004)
        buf[start + i] += attack * sum(a * math.sin(2 * math.pi * f * m * t) * math.exp(-d * t) for m, a, d in PARTIALS)
peak = max(abs(x) for x in buf) or 1.0
with wave.open(sys.argv[1] if len(sys.argv) > 1 else "gong.wav", "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, x / peak * 0.9)) * 32767)) for x in buf))
