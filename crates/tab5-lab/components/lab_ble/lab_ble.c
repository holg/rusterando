/*
 * lab_ble: the Nordic UART Service (NUS) as a NimBLE peripheral.
 *
 * The P4 has no radio: NimBLE's host runs here and talks HCI to the
 * ESP32-C6's Bluetooth controller through esp_hosted (VHCI over the same
 * SDIO link as Wi-Fi; CONFIG_ESP_HOSTED_ENABLE_BT_NIMBLE). The setup
 * follows esp_hosted's host_nimble_bleprph_host_only_vhci example.
 *
 * NUS (any BLE terminal app knows it):
 *   service 6E400001-B5A3-F393-E0A9-E50E24DCCA9E
 *   RX      6E400002-…  the phone writes here   → lab_ble_read()
 *   TX      6E400003-…  we notify here          ← lab_ble_send()
 */

#include <stdio.h>
#include <string.h>
#include <sys/param.h>

#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/stream_buffer.h"
#include "freertos/task.h"
#include "host/ble_hs.h"
#include "host/util/util.h"
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"
#include "nvs_flash.h"
#include "services/gap/ble_svc_gap.h"
#include "services/gatt/ble_svc_gatt.h"

#include "lab_ble.h"

/* esp_hosted 2.x (the JC1060P470's C6 firmware) wants its Bluetooth
 * controller switched on over RPC before the host stack starts; 1.x (the
 * Tab5's) has it running already and lacks these calls. */
#if __has_include("esp_hosted_misc.h")
#include "esp_hosted.h"
#include "esp_hosted_misc.h"
#define LAB_BLE_HOSTED_BT_RPC 1
#endif

static const char *TAG = "lab_ble";

/* 128-bit UUIDs, least significant byte first. */
static const ble_uuid128_t NUS_SVC = BLE_UUID128_INIT(
    0x9e, 0xca, 0xdc, 0x24, 0x0e, 0xe5, 0xa9, 0xe0, 0x93, 0xf3, 0xa3, 0xb5, 0x01, 0x00, 0x40, 0x6e);
static const ble_uuid128_t NUS_RX = BLE_UUID128_INIT(
    0x9e, 0xca, 0xdc, 0x24, 0x0e, 0xe5, 0xa9, 0xe0, 0x93, 0xf3, 0xa3, 0xb5, 0x02, 0x00, 0x40, 0x6e);
static const ble_uuid128_t NUS_TX = BLE_UUID128_INIT(
    0x9e, 0xca, 0xdc, 0x24, 0x0e, 0xe5, 0xa9, 0xe0, 0x93, 0xf3, 0xa3, 0xb5, 0x03, 0x00, 0x40, 0x6e);

/* Received bytes wait here for lab_ble_read(). */
#define RX_BUFFER 4096

static char s_name[32];
static char s_addr[18];
static uint8_t s_own_addr_type;
static volatile bool s_ready;
static volatile uint16_t s_conn = BLE_HS_CONN_HANDLE_NONE;
static volatile bool s_subscribed;
static volatile uint32_t s_session;
static uint16_t s_tx_handle;
static StreamBufferHandle_t s_rx;

static int nus_access(uint16_t conn_handle, uint16_t attr_handle, struct ble_gatt_access_ctxt *ctxt,
                      void *arg)
{
    (void)conn_handle;
    (void)attr_handle;
    (void)arg;
    switch (ctxt->op) {
    case BLE_GATT_ACCESS_OP_WRITE_CHR: {
        uint8_t buf[128];
        uint16_t len = OS_MBUF_PKTLEN(ctxt->om);
        for (uint16_t off = 0; off < len;) {
            uint16_t n = MIN(sizeof(buf), (size_t)(len - off));
            if (os_mbuf_copydata(ctxt->om, off, n, buf) != 0) {
                return BLE_ATT_ERR_UNLIKELY;
            }
            if (xStreamBufferSend(s_rx, buf, n, 0) < n) {
                ESP_LOGW(TAG, "receive buffer full, bytes dropped");
            }
            off += n;
        }
        return 0;
    }
    case BLE_GATT_ACCESS_OP_READ_CHR:
        return 0; /* TX read: empty, the data comes as notifications */
    default:
        return BLE_ATT_ERR_UNLIKELY;
    }
}

static const struct ble_gatt_svc_def s_services[] = {
    {
        .type = BLE_GATT_SVC_TYPE_PRIMARY,
        .uuid = &NUS_SVC.u,
        .characteristics =
            (struct ble_gatt_chr_def[]){
                {
                    .uuid = &NUS_RX.u,
                    .access_cb = nus_access,
                    .flags = BLE_GATT_CHR_F_WRITE | BLE_GATT_CHR_F_WRITE_NO_RSP,
                },
                {
                    .uuid = &NUS_TX.u,
                    .access_cb = nus_access,
                    .val_handle = &s_tx_handle,
                    .flags = BLE_GATT_CHR_F_NOTIFY,
                },
                {0},
            },
    },
    {0},
};

static void advertise(void);

static int gap_event(struct ble_gap_event *event, void *arg)
{
    (void)arg;
    switch (event->type) {
    case BLE_GAP_EVENT_CONNECT:
        if (event->connect.status == 0) {
            s_conn = event->connect.conn_handle;
            s_session++;
            ESP_LOGI(TAG, "connected (handle %d)", s_conn);
        } else {
            ESP_LOGW(TAG, "connection failed (%d)", event->connect.status);
            advertise();
        }
        return 0;
    case BLE_GAP_EVENT_DISCONNECT:
        ESP_LOGI(TAG, "disconnected (reason 0x%x)", event->disconnect.reason);
        s_conn = BLE_HS_CONN_HANDLE_NONE;
        s_subscribed = false;
        advertise();
        return 0;
    case BLE_GAP_EVENT_SUBSCRIBE:
        if (event->subscribe.attr_handle == s_tx_handle) {
            s_subscribed = event->subscribe.cur_notify;
            ESP_LOGI(TAG, "notifications %s", s_subscribed ? "on" : "off");
        }
        return 0;
    case BLE_GAP_EVENT_MTU:
        ESP_LOGI(TAG, "MTU %d", event->mtu.value);
        return 0;
    case BLE_GAP_EVENT_ADV_COMPLETE:
        advertise();
        return 0;
    default:
        return 0;
    }
}

/* The NUS UUID in the advertisement (phones filter on it), the name in
 * the scan response (both together don't fit 31 bytes). */
static void advertise(void)
{
    struct ble_hs_adv_fields fields = {0};
    fields.flags = BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP;
    fields.uuids128 = &NUS_SVC;
    fields.num_uuids128 = 1;
    fields.uuids128_is_complete = 1;
    int rc = ble_gap_adv_set_fields(&fields);
    if (rc != 0) {
        ESP_LOGE(TAG, "advertising data: %d", rc);
        return;
    }
    struct ble_hs_adv_fields rsp = {0};
    rsp.name = (const uint8_t *)s_name;
    rsp.name_len = strlen(s_name);
    rsp.name_is_complete = 1;
    rc = ble_gap_adv_rsp_set_fields(&rsp);
    if (rc != 0) {
        ESP_LOGE(TAG, "scan response: %d", rc);
        return;
    }
    struct ble_gap_adv_params params = {0};
    params.conn_mode = BLE_GAP_CONN_MODE_UND;
    params.disc_mode = BLE_GAP_DISC_MODE_GEN;
    rc = ble_gap_adv_start(s_own_addr_type, NULL, BLE_HS_FOREVER, &params, gap_event, NULL);
    if (rc != 0 && rc != BLE_HS_EALREADY) {
        ESP_LOGE(TAG, "advertising: %d", rc);
    }
}

/* Host and the C6's controller agree: the link works. */
static void on_sync(void)
{
    int rc = ble_hs_util_ensure_addr(0);
    if (rc == 0) {
        rc = ble_hs_id_infer_auto(0, &s_own_addr_type);
    }
    if (rc != 0) {
        ESP_LOGE(TAG, "no usable address: %d", rc);
        return;
    }
    uint8_t a[6] = {0};
    ble_hs_id_copy_addr(s_own_addr_type, a, NULL);
    snprintf(s_addr, sizeof(s_addr), "%02x:%02x:%02x:%02x:%02x:%02x", a[5], a[4], a[3], a[2], a[1],
             a[0]);
    s_ready = true;
    ESP_LOGI(TAG, "controller ready (ESP32-C6 via esp_hosted), address %s, advertising as %s", s_addr,
             s_name);
    advertise();
}

static void on_reset(int reason)
{
    s_ready = false;
    ESP_LOGW(TAG, "host reset, reason %d", reason);
}

static void host_task(void *param)
{
    (void)param;
    nimble_port_run(); /* returns only after nimble_port_stop() */
    nimble_port_freertos_deinit();
}

esp_err_t lab_ble_start(const char *name)
{
    strlcpy(s_name, name, sizeof(s_name));
    /* NimBLE may keep data in NVS. Never erased here: the partition holds
     * the kitchen firmware's Wi-Fi and login too. */
    esp_err_t err = nvs_flash_init();
    if (err != ESP_OK) {
        ESP_LOGW(TAG, "nvs: %s (continuing without)", esp_err_to_name(err));
    }
    s_rx = xStreamBufferCreate(RX_BUFFER, 1);
    if (s_rx == NULL) {
        return ESP_ERR_NO_MEM;
    }
#ifdef LAB_BLE_HOSTED_BT_RPC
    esp_hosted_connect_to_slave();
    if (esp_hosted_bt_controller_init() != ESP_OK) {
        ESP_LOGW(TAG, "C6 Bluetooth controller: init failed (running already?)");
    }
    if (esp_hosted_bt_controller_enable() != ESP_OK) {
        ESP_LOGW(TAG, "C6 Bluetooth controller: enable failed (running already?)");
    }
#endif
    err = nimble_port_init();
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "nimble_port_init: %s", esp_err_to_name(err));
        return err;
    }
    ble_hs_cfg.sync_cb = on_sync;
    ble_hs_cfg.reset_cb = on_reset;
    ble_svc_gap_init();
    ble_svc_gatt_init();
    int rc = ble_gatts_count_cfg(s_services);
    if (rc == 0) {
        rc = ble_gatts_add_svcs(s_services);
    }
    if (rc != 0) {
        ESP_LOGE(TAG, "GATT services: %d", rc);
        return ESP_FAIL;
    }
    ble_svc_gap_device_name_set(s_name);
    nimble_port_freertos_init(host_task);
    ESP_LOGI(TAG, "host started, waiting for the controller");
    return ESP_OK;
}

bool lab_ble_ready(void)
{
    return s_ready;
}

bool lab_ble_connected(void)
{
    return s_conn != BLE_HS_CONN_HANDLE_NONE && s_subscribed;
}

uint32_t lab_ble_session(void)
{
    return s_session;
}

size_t lab_ble_read(uint8_t *buf, size_t cap)
{
    return s_rx != NULL ? xStreamBufferReceive(s_rx, buf, cap, 0) : 0;
}

esp_err_t lab_ble_send(const uint8_t *data, size_t len)
{
    uint16_t conn = s_conn;
    if (conn == BLE_HS_CONN_HANDLE_NONE || !s_subscribed) {
        return ESP_ERR_INVALID_STATE;
    }
    uint16_t mtu = ble_att_mtu(conn);
    size_t chunk = mtu > 3 ? mtu - 3 : 20;
    int stalls = 0;
    for (size_t off = 0; off < len;) {
        size_t n = MIN(chunk, len - off);
        struct os_mbuf *om = ble_hs_mbuf_from_flat(data + off, n);
        int rc = om != NULL ? ble_gatts_notify_custom(conn, s_tx_handle, om) : BLE_HS_ENOMEM;
        if (rc == BLE_HS_ENOMEM) {
            /* Out of buffers until the controller has sent some: wait. */
            if (++stalls > 100) {
                return ESP_ERR_NO_MEM;
            }
            vTaskDelay(pdMS_TO_TICKS(5));
            continue;
        }
        if (rc != 0) {
            return ESP_FAIL;
        }
        stalls = 0;
        off += n;
    }
    return ESP_OK;
}

const char *lab_ble_address(void)
{
    return s_addr;
}

/*
 * The colour e-ink frame (../e-ink-color, BLE protocol v1,
 * docs/ble-protocol.md): here the Tab5 is the central and sends the frame
 * one whole picture, as the iPhone app does. Needs NimBLE's central and
 * observer roles (kds-tab5's sdkconfig); tab5-lab builds without them and
 * gets LAB_EINK_UNSUPPORTED.
 *
 * Flow control: the frame writes every Data chunk straight to the panel.
 * Most chunks go as write-without-response; every EINK_ACK_EVERY-th one as
 * a write with response, whose answer comes only after the frame took all
 * before it — so it never has more than that many chunks waiting.
 */
#if CONFIG_BT_NIMBLE_ROLE_CENTRAL && CONFIG_BT_NIMBLE_ROLE_OBSERVER && CONFIG_BT_NIMBLE_GATT_CLIENT

#include "freertos/event_groups.h"
#include "freertos/queue.h"

/* e1a7000N-5bdd-4a32-89d6-ca4420974f67, least significant byte first. */
#define EINK_UUID(n)                                                                                \
    BLE_UUID128_INIT(0x67, 0x4f, 0x97, 0x20, 0x44, 0xca, 0xd6, 0x89, 0x32, 0x4a, 0xdd, 0x5b, (n), 0x00, \
                     0xa7, 0xe1)
static const ble_uuid128_t EINK_SVC = EINK_UUID(0x01);
static const ble_uuid128_t EINK_CONTROL = EINK_UUID(0x02);
static const ble_uuid128_t EINK_DATA = EINK_UUID(0x03);
static const ble_uuid128_t EINK_STATUS = EINK_UUID(0x04);

#define EINK_BEGIN 0x01
#define EINK_COMMIT 0x02
#define EINK_READY 0x02
#define EINK_DONE 0x04
#define EINK_ERRORS 0x80

#define EINK_ACK_EVERY 8
/* Largest Data chunk: one link-layer packet with data length extension
 * (251 − 4 L2CAP − 3 ATT); the frame takes up to 252. */
#define EINK_CHUNK_MAX 244

#define EV_FOUND BIT0     /* the frame's advertisement seen */
#define EV_SCAN_END BIT1  /* the scan ran out */
#define EV_UP BIT2        /* connect finished (s_ek.rc: 0 = connected) */
#define EV_DOWN BIT3      /* the link is gone */
#define EV_PROC BIT4      /* a GATT procedure finished (s_ek.rc) */

static struct {
    char name[32];
    ble_addr_t addr;
    volatile uint16_t conn;
    volatile int rc;
    uint16_t svc_start, svc_end;
    uint16_t control, data, status, status_end, cccd;
} s_ek;
static EventGroupHandle_t s_ek_ev;
static QueueHandle_t s_ek_status; /* the frame's Status notifications, 5 bytes each */
static volatile bool s_ek_busy;

static void ek_done(int rc)
{
    s_ek.rc = rc;
    xEventGroupSetBits(s_ek_ev, EV_PROC);
}

static int ek_gap_event(struct ble_gap_event *event, void *arg)
{
    (void)arg;
    switch (event->type) {
    case BLE_GAP_EVENT_DISC: {
        struct ble_hs_adv_fields f;
        size_t want = strlen(s_ek.name);
        if (ble_hs_adv_parse_fields(&f, event->disc.data, event->disc.length_data) == 0 &&
            f.name != NULL && f.name_len == want && memcmp(f.name, s_ek.name, want) == 0) {
            s_ek.addr = event->disc.addr;
            xEventGroupSetBits(s_ek_ev, EV_FOUND);
        }
        return 0;
    }
    case BLE_GAP_EVENT_DISC_COMPLETE:
        xEventGroupSetBits(s_ek_ev, EV_SCAN_END);
        return 0;
    case BLE_GAP_EVENT_CONNECT:
        if (event->connect.status == 0) {
            s_ek.conn = event->connect.conn_handle;
            s_ek.rc = 0;
        } else {
            s_ek.rc = event->connect.status;
        }
        xEventGroupSetBits(s_ek_ev, EV_UP);
        return 0;
    case BLE_GAP_EVENT_DISCONNECT:
        ESP_LOGI(TAG, "e-ink frame: disconnected (reason 0x%x)", event->disconnect.reason);
        s_ek.conn = BLE_HS_CONN_HANDLE_NONE;
        xEventGroupSetBits(s_ek_ev, EV_DOWN | EV_PROC | EV_UP);
        return 0;
    case BLE_GAP_EVENT_NOTIFY_RX:
        if (event->notify_rx.attr_handle == s_ek.status && OS_MBUF_PKTLEN(event->notify_rx.om) == 5) {
            uint8_t v[5];
            if (os_mbuf_copydata(event->notify_rx.om, 0, 5, v) == 0) {
                xQueueSend(s_ek_status, v, 0);
            }
        }
        return 0; /* the host frees the mbuf */
    case BLE_GAP_EVENT_MTU:
        ESP_LOGI(TAG, "e-ink frame: MTU %d", event->mtu.value);
        return 0;
    default:
        return 0; /* connection updates the frame asks for: accepted */
    }
}

static int ek_svc_cb(uint16_t conn, const struct ble_gatt_error *error, const struct ble_gatt_svc *svc,
                     void *arg)
{
    (void)conn;
    (void)arg;
    if (error->status == 0 && svc != NULL) {
        s_ek.svc_start = svc->start_handle;
        s_ek.svc_end = svc->end_handle;
        return 0;
    }
    ek_done(error->status == BLE_HS_EDONE ? 0 : error->status);
    return 0;
}

static int ek_chr_cb(uint16_t conn, const struct ble_gatt_error *error, const struct ble_gatt_chr *chr,
                     void *arg)
{
    (void)conn;
    (void)arg;
    if (error->status == 0 && chr != NULL) {
        /* Characteristics come in handle order: the one after Status ends it. */
        if (s_ek.status != 0 && s_ek.status_end == 0) {
            s_ek.status_end = chr->def_handle - 1;
        }
        if (ble_uuid_cmp(&chr->uuid.u, &EINK_CONTROL.u) == 0) {
            s_ek.control = chr->val_handle;
        } else if (ble_uuid_cmp(&chr->uuid.u, &EINK_DATA.u) == 0) {
            s_ek.data = chr->val_handle;
        } else if (ble_uuid_cmp(&chr->uuid.u, &EINK_STATUS.u) == 0) {
            s_ek.status = chr->val_handle;
        }
        return 0;
    }
    ek_done(error->status == BLE_HS_EDONE ? 0 : error->status);
    return 0;
}

static int ek_dsc_cb(uint16_t conn, const struct ble_gatt_error *error, uint16_t chr_val_handle,
                     const struct ble_gatt_dsc *dsc, void *arg)
{
    (void)conn;
    (void)chr_val_handle;
    (void)arg;
    if (error->status == 0 && dsc != NULL) {
        if (s_ek.cccd == 0 && ble_uuid_u16(&dsc->uuid.u) == BLE_GATT_DSC_CLT_CFG_UUID16) {
            s_ek.cccd = dsc->handle;
        }
        return 0;
    }
    ek_done(error->status == BLE_HS_EDONE ? 0 : error->status);
    return 0;
}

static int ek_write_cb(uint16_t conn, const struct ble_gatt_error *error, struct ble_gatt_attr *attr,
                       void *arg)
{
    (void)conn;
    (void)attr;
    (void)arg;
    ek_done(error->status);
    return 0;
}

static int ek_mtu_cb(uint16_t conn, const struct ble_gatt_error *error, uint16_t mtu, void *arg)
{
    (void)conn;
    (void)mtu;
    (void)arg;
    ek_done(error->status);
    return 0;
}

/* Wait for the GATT procedure just started (`rc` its start result). */
static int ek_wait_proc(int rc, uint32_t ms)
{
    if (rc != 0) {
        return rc;
    }
    EventBits_t bits = xEventGroupWaitBits(s_ek_ev, EV_PROC, pdTRUE, pdFALSE, pdMS_TO_TICKS(ms));
    if ((bits & EV_PROC) == 0) {
        return BLE_HS_ETIMEOUT;
    }
    if (s_ek.conn == BLE_HS_CONN_HANDLE_NONE) {
        return BLE_HS_ENOTCONN;
    }
    return s_ek.rc;
}

/* A write with response to `handle`, waited for. */
static int ek_write(uint16_t handle, const uint8_t *data, size_t len, uint32_t ms)
{
    xEventGroupClearBits(s_ek_ev, EV_PROC);
    return ek_wait_proc(ble_gattc_write_flat(s_ek.conn, handle, data, len, ek_write_cb, NULL), ms);
}

static int ek_control(uint8_t op, uint32_t arg)
{
    uint8_t v[5] = {op, arg & 0xff, (arg >> 8) & 0xff, (arg >> 16) & 0xff, (arg >> 24) & 0xff};
    return ek_write(s_ek.control, v, sizeof(v), 5000);
}

/* Wait for the Status `want`, passing over the others (PREPARING,
 * REFRESHING, a stray IDLE); the frame's errors end it. LAB_EINK_OK,
 * LAB_EINK_FRAME (state in *detail), LAB_EINK_LINK or LAB_EINK_TIMEOUT. */
static int ek_await(uint8_t want, uint32_t ms, int32_t *detail)
{
    TickType_t until = xTaskGetTickCount() + pdMS_TO_TICKS(ms);
    for (;;) {
        if (s_ek.conn == BLE_HS_CONN_HANDLE_NONE) {
            return LAB_EINK_LINK;
        }
        TickType_t now = xTaskGetTickCount();
        if ((int32_t)(until - now) <= 0) {
            return LAB_EINK_TIMEOUT;
        }
        uint8_t v[5];
        TickType_t step = MIN(until - now, pdMS_TO_TICKS(200));
        if (xQueueReceive(s_ek_status, v, step) != pdTRUE) {
            continue;
        }
        uint32_t arg = v[1] | (v[2] << 8) | (v[3] << 16) | ((uint32_t)v[4] << 24);
        ESP_LOGI(TAG, "e-ink frame: status 0x%02x (%lu)", v[0], (unsigned long)arg);
        if (v[0] == want) {
            return LAB_EINK_OK;
        }
        if (v[0] >= EINK_ERRORS) {
            *detail = v[0];
            return LAB_EINK_FRAME;
        }
    }
}

/* An error from the frame waiting in the queue (it reports bad data at once). */
static bool ek_frame_error(int32_t *detail)
{
    uint8_t v[5];
    while (xQueueReceive(s_ek_status, v, 0) == pdTRUE) {
        if (v[0] >= EINK_ERRORS) {
            *detail = v[0];
            return true;
        }
    }
    return false;
}

/* Connected: find the service, subscribe, send. */
static int ek_send(const uint8_t *frame, size_t len, uint32_t crc, int32_t *detail)
{
    int rc;
    xEventGroupClearBits(s_ek_ev, EV_PROC);
    rc = ek_wait_proc(ble_gattc_exchange_mtu(s_ek.conn, ek_mtu_cb, NULL), 5000);
    if (rc != 0) {
        ESP_LOGW(TAG, "e-ink frame: MTU exchange %d (continuing)", rc);
        if (s_ek.conn == BLE_HS_CONN_HANDLE_NONE) {
            return LAB_EINK_LINK;
        }
    }
    xEventGroupClearBits(s_ek_ev, EV_PROC);
    rc = ek_wait_proc(ble_gattc_disc_svc_by_uuid(s_ek.conn, &EINK_SVC.u, ek_svc_cb, NULL), 5000);
    if (rc != 0 || s_ek.svc_start == 0) {
        *detail = rc;
        return LAB_EINK_SERVICE;
    }
    xEventGroupClearBits(s_ek_ev, EV_PROC);
    rc = ek_wait_proc(ble_gattc_disc_all_chrs(s_ek.conn, s_ek.svc_start, s_ek.svc_end, ek_chr_cb, NULL),
                      5000);
    if (rc != 0 || s_ek.control == 0 || s_ek.data == 0 || s_ek.status == 0) {
        *detail = rc;
        return LAB_EINK_SERVICE;
    }
    if (s_ek.status_end == 0) {
        s_ek.status_end = s_ek.svc_end;
    }
    if (s_ek.status_end > s_ek.status) {
        xEventGroupClearBits(s_ek_ev, EV_PROC);
        rc = ek_wait_proc(
            ble_gattc_disc_all_dscs(s_ek.conn, s_ek.status, s_ek.status_end, ek_dsc_cb, NULL), 5000);
    }
    if (rc != 0 || s_ek.cccd == 0) {
        *detail = rc;
        return LAB_EINK_SERVICE;
    }
    static const uint8_t notify_on[2] = {0x01, 0x00};
    rc = ek_write(s_ek.cccd, notify_on, sizeof(notify_on), 5000);
    if (rc != 0) {
        *detail = rc;
        return LAB_EINK_LINK;
    }

    xQueueReset(s_ek_status);
    rc = ek_control(EINK_BEGIN, len);
    if (rc != 0) {
        *detail = rc;
        return LAB_EINK_LINK;
    }
    /* PREPARING (panel waking up), then READY. Waits as long as the frame
     * does: its panel driver gives up on BUSY after 60 s and then reports
     * ERR_PANEL — hanging up sooner cut it off mid-start, and the next try
     * found the panel still busy. */
    rc = ek_await(EINK_READY, 75000, detail);
    if (rc != LAB_EINK_OK) {
        return rc;
    }

    uint16_t mtu = ble_att_mtu(s_ek.conn);
    size_t chunk = mtu > 23 ? MIN((size_t)(mtu - 3), (size_t)EINK_CHUNK_MAX) : 20;
    ESP_LOGI(TAG, "e-ink frame: sending %u bytes in %u-byte chunks", (unsigned)len, (unsigned)chunk);
    unsigned n = 0;
    for (size_t off = 0; off < len; n++) {
        size_t c = MIN(chunk, len - off);
        if (n % EINK_ACK_EVERY == EINK_ACK_EVERY - 1 || off + c == len) {
            rc = ek_write(s_ek.data, frame + off, c, 10000);
        } else {
            int stalls = 0;
            while ((rc = ble_gattc_write_no_rsp_flat(s_ek.conn, s_ek.data, frame + off, c)) == BLE_HS_ENOMEM &&
                   ++stalls < 1000) {
                /* Out of buffers until the controller has sent some. */
                vTaskDelay(pdMS_TO_TICKS(2));
            }
        }
        if (rc != 0) {
            *detail = rc;
            return LAB_EINK_LINK;
        }
        if (ek_frame_error(detail)) {
            return LAB_EINK_FRAME;
        }
        off += c;
    }

    rc = ek_control(EINK_COMMIT, crc);
    if (rc != 0) {
        *detail = rc;
        return LAB_EINK_LINK;
    }
    /* REFRESHING (12 to 25 s), then DONE; the protocol allows 60 s. */
    return ek_await(EINK_DONE, 60000, detail);
}

int lab_ble_eink_push(const char *name, const uint8_t *frame, size_t len, uint32_t crc, uint32_t scan_ms,
                      int32_t *detail)
{
    *detail = 0;
    if (!s_ready) {
        return LAB_EINK_NOT_READY;
    }
    if (s_ek_ev == NULL) {
        s_ek_ev = xEventGroupCreate();
        s_ek_status = xQueueCreate(16, 5);
        if (s_ek_ev == NULL || s_ek_status == NULL) {
            return LAB_EINK_NOT_READY;
        }
    }
    if (s_ek_busy) {
        return LAB_EINK_BUSY;
    }
    s_ek_busy = true;
    memset(&s_ek, 0, sizeof(s_ek));
    s_ek.conn = BLE_HS_CONN_HANDLE_NONE;
    strlcpy(s_ek.name, name, sizeof(s_ek.name));
    xEventGroupClearBits(s_ek_ev, 0xff);
    xQueueReset(s_ek_status);

    /* Active scan: the name is in the frame's scan response. */
    struct ble_gap_disc_params scan = {0};
    scan.filter_duplicates = 1;
    int result = LAB_EINK_NOT_FOUND;
    int rc = ble_gap_disc(s_own_addr_type, scan_ms, &scan, ek_gap_event, NULL);
    if (rc != 0) {
        ESP_LOGW(TAG, "e-ink frame: scan %d", rc);
        *detail = rc;
        result = LAB_EINK_CONNECT;
        goto out;
    }
    EventBits_t bits = xEventGroupWaitBits(s_ek_ev, EV_FOUND | EV_SCAN_END, pdFALSE, pdFALSE,
                                           pdMS_TO_TICKS(scan_ms + 1000));
    ble_gap_disc_cancel();
    if ((bits & EV_FOUND) == 0) {
        goto out;
    }
    ESP_LOGI(TAG, "e-ink frame: %s found, connecting", s_ek.name);

    /* 15–30 ms interval as the frame asks for; 4 s supervision timeout. */
    struct ble_gap_conn_params params = {
        .scan_itvl = 0x0010,
        .scan_window = 0x0010,
        .itvl_min = 12,
        .itvl_max = 24,
        .latency = 0,
        .supervision_timeout = 400,
        .min_ce_len = 0,
        .max_ce_len = 0,
    };
    xEventGroupClearBits(s_ek_ev, EV_UP | EV_DOWN);
    rc = ble_gap_connect(s_own_addr_type, &s_ek.addr, 10000, &params, ek_gap_event, NULL);
    if (rc == 0) {
        bits = xEventGroupWaitBits(s_ek_ev, EV_UP, pdTRUE, pdFALSE, pdMS_TO_TICKS(11000));
        rc = (bits & EV_UP) == 0 ? BLE_HS_ETIMEOUT : s_ek.rc;
    }
    if (rc != 0 || s_ek.conn == BLE_HS_CONN_HANDLE_NONE) {
        ble_gap_conn_cancel();
        *detail = rc;
        result = LAB_EINK_CONNECT;
        goto out;
    }

    result = ek_send(frame, len, crc, detail);

    /* Done (or not): hang up. A transfer cut off this way counts as CANCEL. */
    if (s_ek.conn != BLE_HS_CONN_HANDLE_NONE) {
        xEventGroupClearBits(s_ek_ev, EV_DOWN);
        if (ble_gap_terminate(s_ek.conn, BLE_ERR_REM_USER_CONN_TERM) == 0) {
            xEventGroupWaitBits(s_ek_ev, EV_DOWN, pdTRUE, pdFALSE, pdMS_TO_TICKS(3000));
        }
    }
out:
    s_ek_busy = false;
    return result;
}

#else

int lab_ble_eink_push(const char *name, const uint8_t *frame, size_t len, uint32_t crc, uint32_t scan_ms,
                      int32_t *detail)
{
    (void)name;
    (void)frame;
    (void)len;
    (void)crc;
    (void)scan_ms;
    *detail = 0;
    return LAB_EINK_UNSUPPORTED;
}

#endif
