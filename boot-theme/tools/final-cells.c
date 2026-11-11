/* Print the terminal frame of every TTFX logo effect named on stdin as
 * "effect <name>" followed by one "codepoint fg_rgba" line per cell. */
#include <stdio.h>
#include <string.h>
#include "ttfx_plymouth.h"
#include "embedded-logo.h"

int main(void)
{
        char name[TTFX_MAX_EFFECT_NAME_BYTES];
        while (fgets(name, sizeof(name), stdin)) {
                name[strcspn(name, "\r\n")] = '\0';
                if (!name[0])
                        continue;
                TtfxEngine *engine = NULL;
                uint8_t looped, finished = 0;
                const TtfxCell *cells;
                size_t count;
                uint32_t width, height;
                if (ttfx_engine_create_with_background(name, 22321466108495961ull,
                                                       ttfx_embedded_logo, ttfx_embedded_logo_len,
                                                       162, 20, 240, 0x1a1b26, &engine))
                        return 2;
                for (int i = 0; i < 200000 && !finished; i++)
                        if (ttfx_engine_draw_logo(engine, TTFX_LOGO_MODE_ONE_TIME, &looped, &finished))
                                return 3;
                if (!finished || ttfx_engine_cells(engine, &cells, &count, &width, &height))
                        return 4;
                printf("effect %s %u %u\n", name, width, height);
                for (size_t i = 0; i < count; i++)
                        printf("%x %08x %x\n", cells[i].codepoint, cells[i].fg_rgba, cells[i].flags);
                ttfx_engine_free(engine);
        }
        return 0;
}
