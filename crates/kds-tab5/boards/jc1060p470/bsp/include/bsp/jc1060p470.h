/*
 * Board package for the Guition JC1060P470 (ESP32-P4, 7" 1024x600 MIPI-DSI
 * panel with JD9165 controller, GT911 touch, IP101 Ethernet, ESP32-C6 for
 * Wi-Fi), as used by kds-tab5.
 *
 * Only what the kitchen display needs, under the function names of
 * Espressif's board packages (bsp_display_new, bsp_touch_new, ...), so the
 * Rust code shared with the M5Stack Tab5 build runs unchanged.
 *
 * Pins: the board's schematic (JC1060P470C_I_W_Y-V1.0), Guition's factory
 * firmware and examples. Ethernet and the C6's SDIO are wired as on
 * Espressif's ESP32-P4 EV board (ESP-IDF's P4 defaults).
 */
#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "esp_err.h"
#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "esp_lcd_types.h"
#include "esp_lcd_mipi_dsi.h"
#include "esp_lcd_touch.h"
#include "esp_netif.h"
#include "esp_codec_dev.h"

#ifdef __cplusplus
extern "C" {
#endif

/* I2C (touch 0x5D/0x14, ES8311 codec 0x18, RX8025T clock 0x32) */
#define BSP_I2C_NUM            (I2C_NUM_1)
#define BSP_I2C_SDA            (GPIO_NUM_7)
#define BSP_I2C_SCL            (GPIO_NUM_8)

/* Display */
#define BSP_LCD_H_RES          (1024)
#define BSP_LCD_V_RES          (600)
#define BSP_LCD_BITS_PER_PIXEL (16)
#define BSP_LCD_MIPI_DSI_LANE_NUM          (2)
/* 720, not Guition's 750: exact DSI timing with ESP-IDF 5.4.4 (bsp_display.c). */
#define BSP_LCD_MIPI_DSI_LANE_BITRATE_MBPS (720)
#define BSP_LCD_BACKLIGHT      (GPIO_NUM_23)
#define BSP_LCD_RST            (GPIO_NUM_0)
/* The MIPI DSI PHY's supply: on-chip LDO channel 3 at 2.5 V. */
#define BSP_MIPI_DSI_PHY_PWR_LDO_CHAN       (3)
#define BSP_MIPI_DSI_PHY_PWR_LDO_VOLTAGE_MV (2500)

/* Touch (GT911 on the I2C bus). Its reset and interrupt lines are wired
 * (GPIO22, GPIO21) but left alone, as in Guition's own code: driving them
 * would pick the I2C address; bsp_touch_new probes both instead. */
#define BSP_LCD_TOUCH_RST      (GPIO_NUM_NC)
#define BSP_LCD_TOUCH_INT      (GPIO_NUM_NC)

/* Audio: ES8311 codec over I2S into the NS4150 amplifier; the speaker
 * socket is MX1.25 2-pin (no speaker comes with the board). */
#define BSP_I2S_NUM            (I2S_NUM_0)
#define BSP_I2S_MCLK           (GPIO_NUM_13)
#define BSP_I2S_SCLK           (GPIO_NUM_12)
#define BSP_I2S_LCLK           (GPIO_NUM_10)
#define BSP_I2S_DOUT           (GPIO_NUM_9)  /* P4 -> codec */
#define BSP_I2S_DSIN           (GPIO_NUM_48) /* codec -> P4 (microphone), unused */
#define BSP_POWER_AMP_IO       (GPIO_NUM_11) /* NS4150 enable, active high */

/* Ethernet: the P4's EMAC, RMII to an IP101 PHY */
#define BSP_ETH_MDC            (31)
#define BSP_ETH_MDIO           (52)
#define BSP_ETH_PHY_RST        (51)
#define BSP_ETH_PHY_ADDR       (1)

/* I2C */
esp_err_t bsp_i2c_init(void);
i2c_master_bus_handle_t bsp_i2c_get_handle(void);

/* Display, as in Espressif's P4 board packages */
typedef struct {
    struct {
        mipi_dsi_phy_clock_source_t phy_clk_src; /* DSI bus config - clock source (0 = default) */
        uint32_t lane_bit_rate_mbps;             /* DSI bus config - lane bit rate */
    } dsi_bus;
} bsp_display_config_t;

/* Panel, DSI bus and backlight PWM; the panel is not switched on and the
 * backlight stays off (esp_lcd_panel_disp_on_off, bsp_display_brightness_set). */
esp_err_t bsp_display_new(const bsp_display_config_t *config, esp_lcd_panel_handle_t *ret_panel,
                          esp_lcd_panel_io_handle_t *ret_io);
esp_err_t bsp_display_brightness_set(int brightness_percent);

/* Touch */
typedef struct {
    void *dummy;
} bsp_touch_config_t;

/* Touch points come in panel pixels (0..1023, 0..599), whatever resolution
 * the GT911's own configuration reports. Needs bsp_i2c_init first. */
esp_err_t bsp_touch_new(const bsp_touch_config_t *config, esp_lcd_touch_handle_t *ret_touch);

/* Audio, as in Espressif's board packages. The codec handle plays 48 kHz /
 * 16 bit / mono after esp_codec_dev_open; the amplifier stays off until
 * bsp_feature_enable(BSP_FEATURE_SPEAKER, true). */
esp_codec_dev_handle_t bsp_audio_codec_speaker_init(void);

typedef enum {
    BSP_FEATURE_SPEAKER,
} bsp_feature_t;

esp_err_t bsp_feature_enable(bsp_feature_t feature, bool enable);

/* USB host (the receipt printer) */
typedef enum bsp_usb_host_power_mode_t {
    BSP_USB_HOST_POWER_MODE_USB_DEV, /* Power from USB DEV port */
} bsp_usb_host_power_mode_t;

esp_err_t bsp_usb_host_start(bsp_usb_host_power_mode_t mode, bool limit_500mA);

/* Ethernet: start the EMAC + IP101 with a DHCP client on a new default
 * Ethernet netif. Needs the default event loop. */
esp_err_t bsp_eth_start(esp_netif_t **ret_netif);

/* Is a cable plugged in with a live link at the other end? */
bool bsp_eth_link_up(void);

#ifdef __cplusplus
}
#endif
