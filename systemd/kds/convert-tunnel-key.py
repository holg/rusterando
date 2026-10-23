#!/usr/bin/env python3
"""Convert the board's Dropbear ed25519 tunnel key to OpenSSH format.

Runs ON the board (the key never leaves it). Needed once OpenSSH replaces
Dropbear — e.g. after `apt-get install autossh` — because the OpenSSH client
cannot read Dropbear's key format and `dropbearconvert` is gone with
Dropbear. Same key, same fingerprint; the server needs no change.

    convert-tunnel-key.py [SRC] [DST]
    # defaults: /etc/rusterando-printer/tunnel_key
    #           /etc/rusterando-printer/tunnel_key.openssh

Verify:  ssh-keygen -y -f DST | ssh-keygen -lf -
"""
import base64
import os
import struct
import sys

SRC = sys.argv[1] if len(sys.argv) > 1 else "/etc/rusterando-printer/tunnel_key"
DST = sys.argv[2] if len(sys.argv) > 2 else "/etc/rusterando-printer/tunnel_key.openssh"


def read_string(buf: bytes, i: int):
    (n,) = struct.unpack(">I", buf[i : i + 4])
    return buf[i + 4 : i + 4 + n], i + 4 + n


def string(x: bytes) -> bytes:
    return struct.pack(">I", len(x)) + x


def main() -> int:
    raw = open(SRC, "rb").read()
    if raw.startswith(b"-----BEGIN OPENSSH"):
        print(f"{SRC} is already in OpenSSH format", file=sys.stderr)
        return 1
    # Dropbear: string "ssh-ed25519", string(seed[32] || pub[32])
    key_type, i = read_string(raw, 0)
    blob, i = read_string(raw, i)
    if key_type != b"ssh-ed25519" or len(blob) != 64 or i != len(raw):
        print(f"{SRC}: not a Dropbear ed25519 key", file=sys.stderr)
        return 1
    seed, pub = blob[:32], blob[32:]
    pub_blob = string(b"ssh-ed25519") + string(pub)
    check = os.urandom(4)
    priv = (
        check
        + check
        + string(b"ssh-ed25519")
        + string(pub)
        + string(seed + pub)
        + string(b"rusterando-tunnel (converted from dropbear)")
    )
    pad = 1
    while len(priv) % 8:
        priv += bytes([pad])
        pad += 1
    body = (
        b"openssh-key-v1\0"
        + string(b"none")
        + string(b"none")
        + string(b"")
        + struct.pack(">I", 1)
        + string(pub_blob)
        + string(priv)
    )
    b64 = base64.b64encode(body).decode()
    pem = (
        "-----BEGIN OPENSSH PRIVATE KEY-----\n"
        + "\n".join(b64[j : j + 70] for j in range(0, len(b64), 70))
        + "\n-----END OPENSSH PRIVATE KEY-----\n"
    )
    fd = os.open(DST, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(pem)
    print(f"wrote {DST}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
