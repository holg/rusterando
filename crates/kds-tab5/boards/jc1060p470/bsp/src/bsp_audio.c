/*
 * Audio out for the gong: the ES8311 codec (I2C 0x18 on the board's bus),
 * fed over I2S, into the NS4150 class-D amplifier (enable GPIO11, active
 * high, about 3 W into 4 ohms) and the speaker socket — MX1.25 2-pin, "3"
 * on Guition's interface sheet, left of the micro-SD slot. No speaker comes
 * with the board. Pins from the board's schematic.
 *
 * The same functions as Espressif's board packages
 * (bsp_audio_codec_speaker_init, bsp_feature_enable), so the Tab5's
 * src/gong.rs plays the gong here unchanged.
 */

#include "driver/gpio.h"
#include "driver/i2s_std.h"
#include "esp_check.h"
#include "esp_codec_dev_defaults.h"
#include "esp_log.h"

#include "bsp/jc1060p470.h"

static const char *TAG = "JC1060P470";

static i2s_chan_handle_t i2s_tx;
static const audio_codec_data_if_t *i2s_data_if;
static bool amp_ready;

static esp_err_t amp_init(void)
{
    if (amp_ready) {
        return ESP_OK;
    }
    const gpio_config_t io = {
        .pin_bit_mask = 1ULL << BSP_POWER_AMP_IO,
        .mode = GPIO_MODE_OUTPUT,
    };
    ESP_RETURN_ON_ERROR(gpio_config(&io), TAG, "amplifier GPIO");
    ESP_RETURN_ON_ERROR(gpio_set_level(BSP_POWER_AMP_IO, 0), TAG, "amplifier off");
    amp_ready = true;
    return ESP_OK;
}

esp_err_t bsp_feature_enable(bsp_feature_t feature, bool enable)
{
    switch (feature) {
    case BSP_FEATURE_SPEAKER:
        ESP_RETURN_ON_ERROR(amp_init(), TAG, "amplifier");
        return gpio_set_level(BSP_POWER_AMP_IO, enable ? 1 : 0);
    default:
        return ESP_ERR_NOT_SUPPORTED;
    }
}

/* I2S transmit channel (48 kHz, 16 bit, mono; the codec reconfigures it
 * on esp_codec_dev_open) and the codec's data interface over it. */
static esp_err_t audio_init(void)
{
    if (i2s_data_if) {
        return ESP_OK;
    }
    i2s_chan_config_t chan_cfg = I2S_CHANNEL_DEFAULT_CONFIG(BSP_I2S_NUM, I2S_ROLE_MASTER);
    chan_cfg.auto_clear = true; /* silence, not the last samples, when idle */
    ESP_RETURN_ON_ERROR(i2s_new_channel(&chan_cfg, &i2s_tx, NULL), TAG, "I2S channel");

    const i2s_std_config_t std_cfg = {
        .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG(48000),
        .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_16BIT, I2S_SLOT_MODE_MONO),
        .gpio_cfg = {
            .mclk = BSP_I2S_MCLK,
            .bclk = BSP_I2S_SCLK,
            .ws = BSP_I2S_LCLK,
            .dout = BSP_I2S_DOUT,
            .din = I2S_GPIO_UNUSED,
        },
    };
    esp_err_t err = i2s_channel_init_std_mode(i2s_tx, &std_cfg);
    if (err == ESP_OK) {
        err = i2s_channel_enable(i2s_tx);
    }
    if (err != ESP_OK) {
        i2s_del_channel(i2s_tx);
        i2s_tx = NULL;
        ESP_LOGE(TAG, "I2S setup: %s", esp_err_to_name(err));
        return err;
    }

    const audio_codec_i2s_cfg_t i2s_cfg = {
        .port = BSP_I2S_NUM,
        .rx_handle = NULL,
        .tx_handle = i2s_tx,
    };
    i2s_data_if = audio_codec_new_i2s_data(&i2s_cfg);
    ESP_RETURN_ON_FALSE(i2s_data_if, ESP_FAIL, TAG, "codec data interface");
    return ESP_OK;
}

esp_codec_dev_handle_t bsp_audio_codec_speaker_init(void)
{
    if (bsp_i2c_init() != ESP_OK || audio_init() != ESP_OK || amp_init() != ESP_OK) {
        return NULL;
    }

    const audio_codec_i2c_cfg_t i2c_cfg = {
        .port = BSP_I2C_NUM,
        .addr = ES8311_CODEC_DEFAULT_ADDR,
        .bus_handle = bsp_i2c_get_handle(),
    };
    const audio_codec_ctrl_if_t *ctrl_if = audio_codec_new_i2c_ctrl(&i2c_cfg);
    ESP_RETURN_ON_FALSE(ctrl_if, NULL, TAG, "codec I2C");
    const audio_codec_gpio_if_t *gpio_if = audio_codec_new_gpio();
    ESP_RETURN_ON_FALSE(gpio_if, NULL, TAG, "codec GPIO");

    const es8311_codec_cfg_t codec_cfg = {
        .ctrl_if = ctrl_if,
        .gpio_if = gpio_if,
        .codec_mode = ESP_CODEC_DEV_WORK_MODE_DAC,
        /* The amplifier is switched by bsp_feature_enable (gong.rs: only
         * while the gong plays), not by the codec driver. */
        .pa_pin = -1,
        .master_mode = false,
        .use_mclk = true,
        .hw_gain = {
            .pa_voltage = 5.0,
            .codec_dac_voltage = 3.3,
        },
    };
    const audio_codec_if_t *codec = es8311_codec_new(&codec_cfg);
    ESP_RETURN_ON_FALSE(codec, NULL, TAG, "ES8311 not answering");

    const esp_codec_dev_cfg_t dev_cfg = {
        .dev_type = ESP_CODEC_DEV_TYPE_OUT,
        .codec_if = codec,
        .data_if = i2s_data_if,
    };
    esp_codec_dev_handle_t dev = esp_codec_dev_new(&dev_cfg);
    if (dev) {
        ESP_LOGI(TAG, "Audio: ES8311 + NS4150 ready (speaker socket MX1.25 2-pin)");
    }
    return dev;
}
