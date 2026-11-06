# Receipt logos rendered on the server — plan

Status (2026-11-06): rollout steps 1–3 implemented (protocol 2.3, server
logo store + migration + per-client wire form, Tab5/7" and STM clients
announcing raster bits), not yet deployed. Step 4 (resvg out of the
Tab5/7" firmware) follows once every server runs 2.3.

### Running the rollout

```bash
# 0. the migration on a copy of the production DB first (./tmp)
DATABASE_URL=sqlite:./tmp/davids-copy.db cargo run -p rusterando-server -- migrate-receipt-logos --dry-run
# 1. deploy the server (backup first, as always) — after closing time
scripts/deploy_to_server.sh -e davids full
#    on the server, in the deploy directory, with the service's environment:
#    ./davidspizzeria-server migrate-receipt-logos --dry-run   # then without
# 2. Tab5 / 7": stage both images, arm on /admin/drucker
PROFILE=release scripts/build_kds_tab5.sh stage
BOARD=jc1060p470 PROFILE=release scripts/build_kds_tab5.sh stage
# 3. STM printer client 0.4.0
scripts/deploy_printer.sh
```

The server log shows at start: `receipt logo ready` per theme slot (or a
warning if artwork doesn't render) and, until the migration ran, how many
stored jobs still carry their logo inline (they print fine either way).
A tunnel client's capability applies from the moment it arrives, i.e. after
the replay at connect: replayed jobs reach the STM as SVG (it keeps resvg).

## Why

On 2026-11-05 the 7" kitchen display restarted on its first real order and
didn't print it: rendering the shop's receipt logo (SVG → 1-bit raster with
resvg) needs 232 KB of stack, the renderer had 128 KB. Fixed for now with a
1 MB stack (16fce31), but the class of problem stays: logo complexity comes
from whoever designs the theme, so on a small device it can't be bounded.

Logos change rarely, the server already decides their size (`target_width`)
and already holds the artwork. Rendering them once on the server and sending
the finished bitmap is simpler, smaller and the same everywhere.

## How it works today

- `kitchen_protocol::receipt::compose` turns a `RasterSlot` into
  `ReceiptLine::RasterSvg { hash, target_width, svg }` (protocol 2.2).
  The shop's composed header logo is ~37 KB of SVG; as a 1-bit raster
  (480×79) it's ~4.7 KB.
- Every `KitchenEvent` is stored in `kitchen_outbox.payload`
  (`[major, minor] + postcard`) and broadcast to all clients. Rows are never
  deleted, so each order since June carries its own copy of the SVG.
- Clients:
  - **STM board** (`rusterando-printer`, SSH tunnel): renders with resvg,
    keeps rendered logos on disk, lists their hashes in `Hello.cached_rasters`;
    the server then sends `RasterSvg` with an *empty* `svg` (a reference) to
    that connection (`ref_cached_rasters`, per connection).
  - **Tab5 / 7"** (WebSocket `/api/kitchen/ws`): no cache handshake, full
    SVG with every order; resvg on the device (1 MB PSRAM stack, ~1.7 MB of
    the 4.9 MB firmware with the bundled Poppins font).
  - **Admin preview**: separate `.1bit.png` twins per slot — can drift from
    what a printer renders.
- Reprints and test prints compose fresh from the database; nothing reads old
  outbox payloads except the replay of unacked rows to reconnecting clients.

## Design

1. **Logo store on the server.** New table `receipt_logos`
   (`hash` PK, `svg`, `target_width`, `width`, `height`, `bits`,
   `created_at`) plus an in-process cache. A logo is rendered once, on first
   use, with the same code the devices use today
   (`receipt_escpos::svg::rasterize`, same dithering → identical pixels), in
   `spawn_blocking`. The hash stays `raster_hash(svg, target_width)`, so
   everything already cached on devices stays valid.

2. **Stored jobs hold references, not SVG.** `compose` puts the logo into the
   store and the outbox row gets `RasterSvg` with an empty `svg` — a form
   protocol 2.2 already defines ("reference"). The SVG is kept once per logo
   instead of once per order.

3. **The wire form is chosen per connection** (the server already rewrites
   messages per tunnel connection):
   - client declared the *raster-bits* capability → `RasterBits` from the store;
   - older client → `RasterSvg` with the SVG filled back in from the store
     (or left as a reference if the tunnel client lists the hash as cached).
   - Hash not in the store (shouldn't happen after the migration) → the logo
     line is dropped, as today on a cache miss; text banners still show
     channel and payment.

4. **Protocol 2.3 (minor).**
   - `ReceiptLine::RasterBits { hash, width, height, bits }`, appended at the
     end of the enum, so every 2.2 payload still decodes unchanged.
   - Capability signalling, added so the server can be deployed first:
     WebSocket devices add `&caps=raster-bits` to the URL (old firmware
     doesn't, and keeps getting SVG); tunnel clients send a new
     `ClientMessage::Capabilities { raster_bits: true }` right after `Hello`
     (`Hello` itself stays unchanged, so old/new decode in both directions).

5. **Migration of existing queues and receipts** (keeps every old job
   printable):
   - One-time, idempotent pass over `kitchen_outbox` in Rust (sqlx migrations
     are SQL-only, and this needs the protocol decoder): for each row that
     decodes as `Print` with a non-empty `svg`, put the logo into
     `receipt_logos` (rendering its bits) and rewrite the row with the empty
     reference. `id`, `created_at`, `acked_at` and the stored version header
     stay exactly as they are; rows of another major or that don't decode are
     left untouched and logged.
   - Covers the live queue (unacked rows, replayed to reconnecting clients)
     and the whole history (acked rows), so any old job can still be printed
     by any client — by new clients as bits, by old ones as SVG.
   - Device-side queues need nothing: the STM's print queue is in memory and
     refilled from the outbox replay; its on-disk logo cache keeps working
     (same hashes); the Tab5/7" keep no queue.
   - Safety, as for every change to Davids' database: run it first on a copy
     of the production DB in `./tmp`, with a dry-run mode that reports rows,
     logos and bytes saved and checks that every migrated row decodes and
     resolves to the same logo bitmap as before; never DROP anything; the
     deploy script's backup runs before it.
   - Side effect: the outbox loses ~37 KB per order since June (and the
     backups shrink accordingly).

6. **Devices.**
   - Tab5 / 7": send the capability, print `RasterBits` directly. In a second
     step remove resvg/usvg/tiny-skia/fontdb, the Poppins font, the renderer
     thread, its 1 MB stack and the warm-up render (firmware ~1.7 MB smaller,
     faster OTA). A `RasterSvg` that still arrives is printed without its logo
     and logged. The desk replay tool (`KDS_REPLAY_JOB`) then fetches a job
     in its wire form from the server instead of using the raw outbox row.
   - STM (`rusterando-printer`): send the capability, print `RasterBits`;
     resvg can go there too once all servers are on 2.3.

7. **Checks moved to where the artwork changes.** At server start, render all
   slot combinations of every theme (today: delivery/pickup × paid/unpaid)
   and log size and time, warning on failure — broken or oversized artwork
   shows up in the deploy log, not on a printer at dinner time.

8. **Later (optional):** the admin preview shows the stored bitmaps instead
   of the `.1bit.png` twins, so preview and print can't differ.

## Rollout (each step on its own, after closing time)

1. **Server + protocol 2.3:** logo store, migration, per-connection wire
   form, capability detection. No client declares the capability yet, so
   every device keeps getting SVG exactly as today. Verify: all devices
   print as before, admin test print with logo, rows migrated, DB size.
2. **Tab5 / 7" firmware:** capability + `RasterBits` (resvg still inside as a
   fallback) → OTA. Verify: Testdruck with logo now comes from the server's
   bitmap; desk replay of job 429.
3. **STM printer client:** capability + `RasterBits` → `deploy_printer.sh`.
4. **Tab5 / 7" firmware without resvg** → OTA.
5. Optional: preview from the store.

## Tests

- kitchen-protocol: `RasterBits` round trip; 2.2 payloads decode unchanged.
- receipt-escpos: for job 429's logo, printing via `RasterBits` produces
  byte-for-byte the same ESC/POS as via `RasterSvg` — the same paper output.
- server: migration on fixture rows (SVG → reference, second run changes
  nothing, `acked_at` kept, undecodable rows untouched); wire form for clients
  with and without the capability, including replay of migrated rows and the
  tunnel's cached-hash references; unknown hash → logo line dropped.
- Desk: replay of job 429 on the 7" through the new path; Testdruck.

## Open decisions

- Run the migration automatically at server start (idempotent, logged) or as
  an explicit one-off command with `--dry-run` first? Proposed: the explicit
  command for the first run on Davids' database, then automatic at start as a
  no-op guard.
- Capability via URL parameter / extra client message (proposed) or inferred
  from the firmware version string?
- Bitmaps always in full to bits-capable clients (proposed; ~5 KB) or also by
  hash reference?
- Outbox pruning stays out of scope.
