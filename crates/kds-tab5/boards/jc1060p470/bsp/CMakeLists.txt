idf_component_register(
    SRCS "src/bsp_i2c.c" "src/bsp_display.c" "src/bsp_touch.c" "src/bsp_usb.c" "src/bsp_eth.c" "src/bsp_audio.c"
    INCLUDE_DIRS "include"
    REQUIRES driver esp_lcd esp_eth esp_netif esp_event
    PRIV_REQUIRES esp_driver_ledc esp_driver_i2c esp_driver_gpio esp_driver_i2s
)
