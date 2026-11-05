/*
 * Ethernet: the P4's own EMAC over RMII to an IP101 PHY (the RJ45 jack),
 * the 50 MHz reference clock coming from the PHY's oscillator. RMII data
 * pins as ESP-IDF's P4 default (Espressif's P4 EV board layout, which this
 * board follows); SMI, PHY reset and address in jc1060p470.h.
 */

#include "esp_check.h"
#include "esp_event.h"
#include "esp_eth.h"
#include "esp_eth_mac_esp.h"
#include "esp_log.h"
#include "esp_netif.h"

#include "bsp/jc1060p470.h"

static const char *TAG = "JC1060P470";

/* The cable's link state, from the driver's events. */
static volatile bool s_link_up;

static void on_eth_event(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    (void)arg;
    (void)base;
    (void)data;
    if (id == ETHERNET_EVENT_CONNECTED) {
        s_link_up = true;
        ESP_LOGI(TAG, "Ethernet link up");
    } else if (id == ETHERNET_EVENT_DISCONNECTED) {
        s_link_up = false;
        ESP_LOGI(TAG, "Ethernet link down");
    }
}

bool bsp_eth_link_up(void)
{
    return s_link_up;
}

esp_err_t bsp_eth_start(esp_netif_t **ret_netif)
{
    ESP_RETURN_ON_FALSE(ret_netif, ESP_ERR_INVALID_ARG, TAG, "arguments");

    eth_mac_config_t mac_config = ETH_MAC_DEFAULT_CONFIG();
    eth_esp32_emac_config_t emac_config = ETH_ESP32_EMAC_DEFAULT_CONFIG();
    emac_config.smi_gpio.mdc_num = BSP_ETH_MDC;
    emac_config.smi_gpio.mdio_num = BSP_ETH_MDIO;
    esp_eth_mac_t *mac = esp_eth_mac_new_esp32(&emac_config, &mac_config);
    ESP_RETURN_ON_FALSE(mac, ESP_FAIL, TAG, "EMAC");

    eth_phy_config_t phy_config = ETH_PHY_DEFAULT_CONFIG();
    phy_config.phy_addr = BSP_ETH_PHY_ADDR;
    phy_config.reset_gpio_num = BSP_ETH_PHY_RST;
    esp_eth_phy_t *phy = esp_eth_phy_new_ip101(&phy_config);
    if (!phy) {
        mac->del(mac);
        ESP_LOGE(TAG, "IP101 PHY");
        return ESP_FAIL;
    }

    esp_eth_handle_t eth = NULL;
    esp_eth_config_t config = ETH_DEFAULT_CONFIG(mac, phy);
    esp_err_t err = esp_eth_driver_install(&config, &eth);
    if (err != ESP_OK) {
        phy->del(phy);
        mac->del(mac);
        ESP_LOGE(TAG, "Ethernet driver: %s", esp_err_to_name(err));
        return err;
    }

    /* Runs before the app's first EspNetif; a second call is a no-op. */
    ESP_RETURN_ON_ERROR(esp_netif_init(), TAG, "netif stack");
    esp_netif_config_t netif_config = ESP_NETIF_DEFAULT_ETH();
    esp_netif_t *netif = esp_netif_new(&netif_config);
    ESP_RETURN_ON_FALSE(netif, ESP_FAIL, TAG, "Ethernet netif");
    ESP_RETURN_ON_ERROR(esp_netif_attach(netif, esp_eth_new_netif_glue(eth)), TAG, "netif glue");
    ESP_RETURN_ON_ERROR(esp_event_handler_register(ETH_EVENT, ESP_EVENT_ANY_ID, on_eth_event, NULL), TAG,
                        "link events");
    ESP_RETURN_ON_ERROR(esp_eth_start(eth), TAG, "Ethernet start");

    ESP_LOGI(TAG, "Ethernet started (IP101, PHY address %d)", BSP_ETH_PHY_ADDR);
    *ret_netif = netif;
    return ESP_OK;
}
