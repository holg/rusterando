// lab_ble: a Bluetooth LE serial link for tab5-lab — the Nordic UART
// Service (NUS) as a NimBLE peripheral. The radio is the ESP32-C6's,
// reached through esp_hosted (VHCI over SDIO); the P4 runs the host stack.
// Also the central side for the colour e-ink frame (lab_ble_eink_push).
//
// Bytes written by the phone to the RX characteristic queue up for
// lab_ble_read(); lab_ble_send() notifies them on the TX characteristic.
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "esp_err.h"

#ifdef __cplusplus
extern "C" {
#endif

// Start the host stack and advertise as `name` (copied). Returns once the
// stack is started; the controller answering shows in lab_ble_ready().
esp_err_t lab_ble_start(const char *name);

// The C6's controller answered (host and controller in sync).
bool lab_ble_ready(void);

// A phone is connected and has notifications on.
bool lab_ble_connected(void);

// Bumped on every new connection, so the caller can greet each one.
uint32_t lab_ble_session(void);

// Received bytes, at most `cap`; 0 when nothing is waiting. Never blocks.
size_t lab_ble_read(uint8_t *buf, size_t cap);

// Send `len` bytes as notifications, split to the connection's MTU.
// ESP_ERR_INVALID_STATE when nobody listens.
esp_err_t lab_ble_send(const uint8_t *data, size_t len);

// The controller's public address, "aa:bb:cc:dd:ee:ff" (empty before ready).
const char *lab_ble_address(void);

// lab_ble_eink_push results.
#define LAB_EINK_OK 0
#define LAB_EINK_UNSUPPORTED 1 // built without NimBLE's central role
#define LAB_EINK_NOT_READY 2   // the controller isn't answering (yet)
#define LAB_EINK_BUSY 3        // another push is running
#define LAB_EINK_NOT_FOUND 4   // no frame advertising that name
#define LAB_EINK_CONNECT 5     // scan or connect failed (*detail: NimBLE code)
#define LAB_EINK_SERVICE 6     // the frame's service or characteristics missing
#define LAB_EINK_LINK 7        // a write failed or the link dropped (*detail: NimBLE code)
#define LAB_EINK_TIMEOUT 8     // the frame didn't answer in time
#define LAB_EINK_FRAME 9       // the frame reported an error (*detail: its state, 0x80…)

// Send one picture to the colour e-ink frame advertising as `name` (BLE
// protocol v1 of ../e-ink-color): scan up to `scan_ms` for it, connect,
// BEGIN, the `len` bytes of `frame`, COMMIT with `crc` (CRC-32, zlib's),
// wait for the panel's refresh, hang up. Blocks, up to about 90 s. Runs
// beside the phone's connection.
int lab_ble_eink_push(const char *name, const uint8_t *frame, size_t len, uint32_t crc, uint32_t scan_ms,
                      int32_t *detail);

// Restart diagnosis (diag.c): keep the last error/warning lines, and any
// line about a restart, in RAM a software restart leaves alone. Install as
// early as possible — after lab_diag_take_last.
void lab_diag_install(void);

// The lines kept from before this start (oldest first, " | " between),
// empty after a power-on or when none were logged. Read once per start.
const char *lab_diag_take_last(void);

#ifdef __cplusplus
}
#endif
