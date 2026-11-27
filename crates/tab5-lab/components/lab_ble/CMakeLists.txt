idf_component_register(
    SRCS "lab_ble.c" "diag.c"
    INCLUDE_DIRS "include"
    REQUIRES bt nvs_flash
    # esp_hosted's headers (the C6's Bluetooth controller calls, 2.x).
    PRIV_REQUIRES espressif__esp_hosted
)
