/*
 * The 7" panel: JD9165 controller on MIPI-DSI (2 lanes), 1024x600
 * landscape, RGB565 from the app, frames copied by the 2D-DMA engine.
 * Backlight PWM on LEDC.
 *
 * Guition ships two panels behind the same JD9165, with different init
 * tables and timing; the wrong one gives a white screen with a noise band.
 * This is the "V2" panel (2026 stock, "V2" after the material number on
 * the rear label): the table below is the one in this unit's factory
 * firmware (read out of the flash backup), identical to Guition's
 * New_Panel driver and ESPHome's JC1060P470-V2 model. The esp_lcd_jd9165
 * driver's own default (sleep out + display on) and its timing macro don't
 * fit either panel.
 *
 * The V1 panel (older stock) would need, from ESPHome's JC1060P470 model:
 * its own table, h 24-40/160/160, v 2-10/21-23/12 (clocks: see below).
 *
 * DSI timing. ESP-IDF 5.4.4 converts each horizontal segment from pixel
 * clocks to DSI lane byte clocks (x lane_rate / pixel_clock / 8) and
 * truncates each one: unless all four products are whole numbers, the
 * DSI line comes out shorter than the pixel line, the two drift apart
 * every frame and the panel keeps losing sync — the whole picture flickers
 * (Guition's 52 MHz request became 60 MHz with 750 Mbps lanes: 1 lane clock
 * short per line, flicker; 48 MHz: 2 short, worse, bottom lines garbled).
 * ESP-IDF 5.5.4, which Guition's factory firmware uses, rounds and
 * compensates instead. So here the ratio is exact: 48 MHz pixel clock
 * (240 MHz / 5) with 720 Mbps lanes = 1.875, and every horizontal value
 * a multiple of 8. Guition's porches unchanged (the panel sees their
 * proportions on the DSI link, as with the factory firmware); frames at
 * 48 MHz / (1344 x 635) = 56 Hz instead of 61. Shortening the front porch
 * to 56 for 61 Hz left the left edge flickering and a faint ghost of
 * bright areas a few pixels to the right.
 */

#include <inttypes.h>

#include "driver/ledc.h"
#include "esp_check.h"
#include "esp_lcd_jd9165.h"
#include "esp_lcd_panel_ops.h"
#include "esp_ldo_regulator.h"
#include "esp_log.h"

#include "bsp/jc1060p470.h"

static const char *TAG = "JC1060P470";

#define LCD_LEDC_CH LEDC_CHANNEL_0
/* 20 kHz: lower PWM frequencies make the picture shimmer while Ethernet
 * runs (seen by the espcontrol project on this board). */
#define LCD_LEDC_HZ 20000

/* Guition's V2 table, as in the factory firmware. Page select 0x30;
 * ends with RGB565 (0x3A 0x55), sleep out + 120 ms, display on + 20 ms. */
static const jd9165_lcd_init_cmd_t panel_v2_init[] = {
    {0x30, (uint8_t[]){0x00}, 1, 0},
    {0xF7, (uint8_t[]){0x49, 0x61, 0x02, 0x00}, 4, 0},
    {0x30, (uint8_t[]){0x01}, 1, 0},
    {0x04, (uint8_t[]){0x0C}, 1, 0},
    {0x05, (uint8_t[]){0x08}, 1, 0},
    {0x0B, (uint8_t[]){0x11}, 1, 0},
    {0x20, (uint8_t[]){0x04}, 1, 0},
    {0x1F, (uint8_t[]){0x05}, 1, 0},
    {0x23, (uint8_t[]){0x38}, 1, 0},
    {0x28, (uint8_t[]){0x18}, 1, 0},
    {0x29, (uint8_t[]){0x29}, 1, 0},
    {0x2A, (uint8_t[]){0x01}, 1, 0},
    {0x2B, (uint8_t[]){0x29}, 1, 0},
    {0x2C, (uint8_t[]){0x01}, 1, 0},
    {0x30, (uint8_t[]){0x02}, 1, 0},
    {0x00, (uint8_t[]){0x05}, 1, 0},
    {0x01, (uint8_t[]){0x22}, 1, 0},
    {0x02, (uint8_t[]){0x08}, 1, 0},
    {0x03, (uint8_t[]){0x12}, 1, 0},
    {0x04, (uint8_t[]){0x16}, 1, 0},
    {0x05, (uint8_t[]){0x64}, 1, 0},
    {0x06, (uint8_t[]){0x00}, 1, 0},
    {0x07, (uint8_t[]){0x00}, 1, 0},
    {0x08, (uint8_t[]){0x78}, 1, 0},
    {0x09, (uint8_t[]){0x00}, 1, 0},
    {0x0A, (uint8_t[]){0x04}, 1, 0},
    {0x0B, (uint8_t[]){0x16, 0x17, 0x0B, 0x0D, 0x0D, 0x0D, 0x11, 0x10, 0x07, 0x07, 0x09}, 11, 0},
    {0x0C, (uint8_t[]){0x09, 0x1E, 0x1E, 0x1C, 0x1C, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D}, 11, 0},
    {0x0D, (uint8_t[]){0x0A, 0x05, 0x0B, 0x0D, 0x0D, 0x0D, 0x11, 0x10, 0x06, 0x06, 0x08}, 11, 0},
    {0x0E, (uint8_t[]){0x08, 0x1F, 0x1F, 0x1D, 0x1D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D}, 11, 0},
    {0x0F, (uint8_t[]){0x0A, 0x05, 0x0D, 0x0B, 0x0D, 0x0D, 0x11, 0x10, 0x1D, 0x1D, 0x1F}, 11, 0},
    {0x10, (uint8_t[]){0x1F, 0x08, 0x08, 0x06, 0x06, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D}, 11, 0},
    {0x11, (uint8_t[]){0x16, 0x17, 0x0D, 0x0B, 0x0D, 0x0D, 0x11, 0x10, 0x1C, 0x1C, 0x1E}, 11, 0},
    {0x12, (uint8_t[]){0x1E, 0x09, 0x09, 0x07, 0x07, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D, 0x0D}, 11, 0},
    {0x13, (uint8_t[]){0x00, 0x00, 0x00, 0x00}, 4, 0},
    {0x14, (uint8_t[]){0x00, 0x00, 0x41, 0x41}, 4, 0},
    {0x15, (uint8_t[]){0x00, 0x00, 0x00, 0x00}, 4, 0},
    {0x17, (uint8_t[]){0x00}, 1, 0},
    {0x18, (uint8_t[]){0x85}, 1, 0},
    {0x19, (uint8_t[]){0x06, 0x09}, 2, 0},
    {0x1A, (uint8_t[]){0x05, 0x08}, 2, 0},
    {0x1B, (uint8_t[]){0x0A, 0x04}, 2, 0},
    {0x26, (uint8_t[]){0x00}, 1, 0},
    {0x27, (uint8_t[]){0x00}, 1, 0},
    {0x30, (uint8_t[]){0x06}, 1, 0},
    {0x12, (uint8_t[]){0x3F, 0x25, 0x27, 0x35, 0x1D, 0x1B, 0x1B, 0x1A, 0x18, 0x0A, 0x2A, 0x21, 0x19, 0x30}, 14, 0},
    {0x13, (uint8_t[]){0x3F, 0x26, 0x27, 0x35, 0x1E, 0x1C, 0x1C, 0x1A, 0x18, 0x0B, 0x2A, 0x21, 0x19, 0x30}, 14, 0},
    {0x30, (uint8_t[]){0x0A}, 1, 0},
    {0x02, (uint8_t[]){0x4F}, 1, 0},
    {0x0B, (uint8_t[]){0x40}, 1, 0},
    {0x30, (uint8_t[]){0x0D}, 1, 0},
    {0x0D, (uint8_t[]){0x04}, 1, 0},
    {0x10, (uint8_t[]){0x05}, 1, 0},
    {0x11, (uint8_t[]){0x0C}, 1, 0},
    {0x12, (uint8_t[]){0x05}, 1, 0},
    {0x13, (uint8_t[]){0x0C}, 1, 0},
    {0x30, (uint8_t[]){0x00}, 1, 0},
    {0x3A, (uint8_t[]){0x55}, 1, 0},
    {0x11, (uint8_t[]){0x00}, 1, 120},
    {0x29, (uint8_t[]){0x00}, 1, 20},
};

static esp_ldo_channel_handle_t phy_ldo;

static esp_err_t brightness_init(void)
{
    const ledc_timer_config_t timer = {
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .duty_resolution = LEDC_TIMER_10_BIT,
        .timer_num = LEDC_TIMER_0,
        .freq_hz = LCD_LEDC_HZ,
        .clk_cfg = LEDC_AUTO_CLK,
    };
    const ledc_channel_config_t channel = {
        .gpio_num = BSP_LCD_BACKLIGHT,
        .speed_mode = LEDC_LOW_SPEED_MODE,
        .channel = LCD_LEDC_CH,
        .intr_type = LEDC_INTR_DISABLE,
        .timer_sel = LEDC_TIMER_0,
        .duty = 0,
        .hpoint = 0,
    };
    ESP_RETURN_ON_ERROR(ledc_timer_config(&timer), TAG, "backlight timer");
    ESP_RETURN_ON_ERROR(ledc_channel_config(&channel), TAG, "backlight channel");
    return ESP_OK;
}

esp_err_t bsp_display_brightness_set(int brightness_percent)
{
    if (brightness_percent > 100) {
        brightness_percent = 100;
    }
    if (brightness_percent < 0) {
        brightness_percent = 0;
    }
    ESP_LOGI(TAG, "Setting LCD backlight: %d%%", brightness_percent);
    /* 10-bit LEDC: 1024 is "always on" (100 % really is no PWM at all). */
    const uint32_t duty = (1024 * brightness_percent) / 100;
    ESP_RETURN_ON_ERROR(ledc_set_duty(LEDC_LOW_SPEED_MODE, LCD_LEDC_CH, duty), TAG, "duty");
    ESP_RETURN_ON_ERROR(ledc_update_duty(LEDC_LOW_SPEED_MODE, LCD_LEDC_CH), TAG, "duty update");
    return ESP_OK;
}

esp_err_t bsp_display_new(const bsp_display_config_t *config, esp_lcd_panel_handle_t *ret_panel,
                          esp_lcd_panel_io_handle_t *ret_io)
{
    ESP_RETURN_ON_FALSE(config && ret_panel && ret_io, ESP_ERR_INVALID_ARG, TAG, "arguments");
    ESP_RETURN_ON_ERROR(brightness_init(), TAG, "backlight");

    if (!phy_ldo) {
        const esp_ldo_channel_config_t ldo = {
            .chan_id = BSP_MIPI_DSI_PHY_PWR_LDO_CHAN,
            .voltage_mv = BSP_MIPI_DSI_PHY_PWR_LDO_VOLTAGE_MV,
        };
        ESP_RETURN_ON_ERROR(esp_ldo_acquire_channel(&ldo, &phy_ldo), TAG, "DSI PHY power");
    }

    esp_lcd_dsi_bus_handle_t bus = NULL;
    const esp_lcd_dsi_bus_config_t bus_config = {
        .bus_id = 0,
        .num_data_lanes = BSP_LCD_MIPI_DSI_LANE_NUM,
        .phy_clk_src = config->dsi_bus.phy_clk_src,
        .lane_bit_rate_mbps = config->dsi_bus.lane_bit_rate_mbps,
    };
    ESP_RETURN_ON_ERROR(esp_lcd_new_dsi_bus(&bus_config, &bus), TAG, "DSI bus");

    esp_lcd_panel_io_handle_t io = NULL;
    const esp_lcd_dbi_io_config_t dbi_config = {
        .virtual_channel = 0,
        .lcd_cmd_bits = 8,
        .lcd_param_bits = 8,
    };
    ESP_RETURN_ON_ERROR(esp_lcd_new_panel_io_dbi(bus, &dbi_config, &io), TAG, "panel IO");

    const esp_lcd_dpi_panel_config_t dpi_config = {
        .virtual_channel = 0,
        .dpi_clk_src = MIPI_DSI_DPI_CLK_SRC_DEFAULT,
        /* See "DSI timing" above: 48 MHz (240 MHz / 5, exact). */
        .dpi_clock_freq_mhz = 48,
        .in_color_format = LCD_COLOR_FMT_RGB565,
        .num_fbs = 1,
        .video_timing = {
            .h_size = BSP_LCD_H_RES,
            .v_size = BSP_LCD_V_RES,
            .hsync_pulse_width = 24,
            .hsync_back_porch = 136,
            .hsync_front_porch = 160,
            .vsync_pulse_width = 2,
            .vsync_back_porch = 21,
            .vsync_front_porch = 12,
        },
        .flags.use_dma2d = true,
    };
    const jd9165_vendor_config_t vendor_config = {
        .init_cmds = panel_v2_init,
        .init_cmds_size = sizeof(panel_v2_init) / sizeof(panel_v2_init[0]),
        .mipi_config = {
            .dsi_bus = bus,
            .dpi_config = &dpi_config,
        },
    };
    const esp_lcd_panel_dev_config_t panel_config = {
        .reset_gpio_num = BSP_LCD_RST,
        .flags.reset_active_high = 0,
        .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
        .bits_per_pixel = BSP_LCD_BITS_PER_PIXEL,
        .vendor_config = (void *)&vendor_config,
    };
    esp_lcd_panel_handle_t panel = NULL;
    ESP_RETURN_ON_ERROR(esp_lcd_new_panel_jd9165(io, &panel_config, &panel), TAG, "JD9165 panel");
    /* Hardware reset, then 120 ms before the table (the driver waits):
     * without that wait the panel stays white after a warm restart. */
    ESP_RETURN_ON_ERROR(esp_lcd_panel_reset(panel), TAG, "panel reset");
    ESP_RETURN_ON_ERROR(esp_lcd_panel_init(panel), TAG, "panel init");

    ESP_LOGI(TAG, "Display initialized: JD9165 V2 panel %dx%d, %" PRIu32 " MHz pixel clock, %" PRIu32 " Mbps lanes",
             BSP_LCD_H_RES, BSP_LCD_V_RES, dpi_config.dpi_clock_freq_mhz, config->dsi_bus.lane_bit_rate_mbps);
    *ret_panel = panel;
    *ret_io = io;
    return ESP_OK;
}
