/*
 * Touch: GT911 on the board's I2C bus, no reset or interrupt line wired
 * (the controller keeps whichever address it came up with: 0x5D or 0x14).
 *
 * Points are scaled to panel pixels from the resolution the GT911's own
 * configuration reports, so the app always sees 0..1023 x 0..599. The
 * first unit reports 1024x600 (no scaling); Guition's factory firmware
 * expects some to report 800x480 ("Touch calibration enabled: 800x480 ->
 * 1024x600").
 */

#include "esp_check.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_touch_gt911.h"
#include "esp_log.h"

#include "bsp/jc1060p470.h"

static const char *TAG = "JC1060P470";

/* GT911 configuration: X/Y output maximum, little-endian. */
#define GT911_REG_X_MAX 0x8048

static uint16_t raw_w = BSP_LCD_H_RES;
static uint16_t raw_h = BSP_LCD_V_RES;

static uint16_t scale(uint16_t v, uint16_t from, uint16_t to)
{
    uint32_t s = (uint32_t)v * to / from;
    return s >= to ? to - 1 : (uint16_t)s;
}

static void to_panel_pixels(esp_lcd_touch_handle_t tp, uint16_t *x, uint16_t *y, uint16_t *strength,
                            uint8_t *point_num, uint8_t max_point_num)
{
    (void)tp;
    (void)strength;
    for (uint8_t i = 0; i < *point_num && i < max_point_num; i++) {
        x[i] = scale(x[i], raw_w, BSP_LCD_H_RES);
        y[i] = scale(y[i], raw_h, BSP_LCD_V_RES);
    }
}

esp_err_t bsp_touch_new(const bsp_touch_config_t *config, esp_lcd_touch_handle_t *ret_touch)
{
    (void)config;
    ESP_RETURN_ON_FALSE(ret_touch, ESP_ERR_INVALID_ARG, TAG, "arguments");
    i2c_master_bus_handle_t bus = bsp_i2c_get_handle();
    ESP_RETURN_ON_FALSE(bus, ESP_ERR_INVALID_STATE, TAG, "bsp_i2c_init first");

    uint16_t addr = ESP_LCD_TOUCH_IO_I2C_GT911_ADDRESS;
    if (i2c_master_probe(bus, addr, 50) != ESP_OK) {
        addr = ESP_LCD_TOUCH_IO_I2C_GT911_ADDRESS_BACKUP;
        ESP_RETURN_ON_ERROR(i2c_master_probe(bus, addr, 50), TAG, "no GT911 at 0x5D or 0x14");
    }

    esp_lcd_panel_io_handle_t io = NULL;
    esp_lcd_panel_io_i2c_config_t io_config = ESP_LCD_TOUCH_IO_I2C_GT911_CONFIG();
    io_config.dev_addr = addr;
    io_config.scl_speed_hz = 400000;
    ESP_RETURN_ON_ERROR(esp_lcd_new_panel_io_i2c(bus, &io_config, &io), TAG, "touch IO");

    uint8_t res[4];
    if (esp_lcd_panel_io_rx_param(io, GT911_REG_X_MAX, res, sizeof res) == ESP_OK) {
        uint16_t w = res[0] | (res[1] << 8);
        uint16_t h = res[2] | (res[3] << 8);
        if (w >= 100 && w <= 4096 && h >= 100 && h <= 4096) {
            raw_w = w;
            raw_h = h;
        }
    }
    ESP_LOGI(TAG, "GT911 at 0x%02x reports %ux%u, points scaled to %dx%d", addr, raw_w, raw_h, BSP_LCD_H_RES,
             BSP_LCD_V_RES);

    /* Only read during the driver's reset/address procedure (no reset line
     * here), but kept alive in case that changes. */
    static esp_lcd_touch_io_gt911_config_t gt911 = { 0 };
    gt911.dev_addr = addr;
    const esp_lcd_touch_config_t tp_cfg = {
        .x_max = BSP_LCD_H_RES,
        .y_max = BSP_LCD_V_RES,
        .rst_gpio_num = BSP_LCD_TOUCH_RST,
        .int_gpio_num = BSP_LCD_TOUCH_INT,
        .levels = {
            .reset = 0,
            .interrupt = 0,
        },
        .process_coordinates = to_panel_pixels,
        .driver_data = &gt911,
    };
    return esp_lcd_touch_new_i2c_gt911(io, &tp_cfg, ret_touch);
}
