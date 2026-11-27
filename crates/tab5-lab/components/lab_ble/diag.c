/*
 * Why did the device restart? A software restart (esp_restart — e.g.
 * esp_hosted's "SDIO slave unresponsive, restart host") logs its reason a
 * moment before; on the USB console those last lines are often lost, and
 * after such a restart this board's USB console stays silent. So the last
 * error and warning lines (and any line about a restart) are kept in RAM
 * that a software restart leaves alone (.noinit), and the next boot shows
 * them (crashlog.rs). Lost on power-off, as they should be.
 *
 * Only those lines are formatted here; every other line passes straight
 * through, costing no stack in the logging task.
 */

#include <stdarg.h>
#include <stdio.h>
#include <string.h>

#include "esp_attr.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"

#include "lab_ble.h"

#define KEEP 3
#define LINE 120
#define MAGIC 0x6c616232u /* "lab2" */

static __NOINIT_ATTR uint32_t s_magic;
static __NOINIT_ATTR uint32_t s_next;
static __NOINIT_ATTR char s_lines[KEEP][LINE];

static vprintf_like_t s_prev;
static portMUX_TYPE s_lock = portMUX_INITIALIZER_UNLOCKED;
static char s_taken[KEEP * (LINE + 3) + 1];

/* An error or warning line ("E (…" / "W (…", after an optional colour
 * code), or a line about a restart. */
static bool worth_keeping(const char *fmt)
{
    const char *p = fmt;
    if (p[0] == '\033') {
        const char *m = strchr(p, 'm');
        if (m != NULL) {
            p = m + 1;
        }
    }
    if ((p[0] == 'E' || p[0] == 'W') && p[1] == ' ' && p[2] == '(') {
        return true;
    }
    return strstr(fmt, "estart") != NULL || strstr(fmt, "reset") != NULL;
}

static int hook(const char *fmt, va_list args)
{
    if (fmt != NULL && worth_keeping(fmt)) {
        char line[LINE];
        va_list copy;
        va_copy(copy, args);
        int n = vsnprintf(line, sizeof(line), fmt, copy);
        va_end(copy);
        if (n > 0) {
            /* Without colour codes and the line end. */
            char *src = line;
            if (src[0] == '\033') {
                char *m = strchr(src, 'm');
                if (m != NULL) {
                    src = m + 1;
                }
            }
            size_t len = strcspn(src, "\033\r\n");
            portENTER_CRITICAL(&s_lock);
            if (s_magic != MAGIC) {
                memset(s_lines, 0, sizeof(s_lines));
                s_next = 0;
                s_magic = MAGIC;
            }
            char *slot = s_lines[s_next % KEEP];
            memcpy(slot, src, len < LINE - 1 ? len : LINE - 1);
            slot[len < LINE - 1 ? len : LINE - 1] = '\0';
            s_next++;
            portEXIT_CRITICAL(&s_lock);
        }
    }
    return s_prev != NULL ? s_prev(fmt, args) : vprintf(fmt, args);
}

const char *lab_diag_take_last(void)
{
    s_taken[0] = '\0';
    if (s_magic == MAGIC && s_next > 0) {
        uint32_t first = s_next > KEEP ? s_next - KEEP : 0;
        for (uint32_t i = first; i < s_next; i++) {
            const char *l = s_lines[i % KEEP];
            if (l[0] == '\0' || strnlen(l, LINE) == LINE) {
                continue;
            }
            if (s_taken[0] != '\0') {
                strlcat(s_taken, " | ", sizeof(s_taken));
            }
            strlcat(s_taken, l, sizeof(s_taken));
        }
    }
    /* Read once: this run starts its own list. */
    s_magic = 0;
    return s_taken;
}

void lab_diag_install(void)
{
    if (s_prev == NULL) {
        s_prev = esp_log_set_vprintf(hook);
    }
}
