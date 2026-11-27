/*
 * The board's I2C bus (touch controller, audio codec), new master API.
 */

#include "esp_check.h"
#include "esp_log.h"

#include "bsp/jc1060p470.h"

static const char *TAG = "JC1060P470";
static i2c_master_bus_handle_t i2c_handle;

esp_err_t bsp_i2c_init(void)
{
    if (i2c_handle) {
        return ESP_OK;
    }
    const i2c_master_bus_config_t cfg = {
        .i2c_port = BSP_I2C_NUM,
        .sda_io_num = BSP_I2C_SDA,
        .scl_io_num = BSP_I2C_SCL,
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .glitch_ignore_cnt = 7,
        .flags.enable_internal_pullup = true,
    };
    ESP_RETURN_ON_ERROR(i2c_new_master_bus(&cfg, &i2c_handle), TAG, "I2C bus");
    return ESP_OK;
}

i2c_master_bus_handle_t bsp_i2c_get_handle(void)
{
    return i2c_handle;
}
