#define _GNU_SOURCE
#include <assert.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <ply-pixel-buffer.h>
#include "state.h"

static uint64_t little(const uint8_t *p, size_t n)
{
        uint64_t value = 0;
        for (size_t i = 0; i < n; i++) value |= (uint64_t)p[i] << (8 * i);
        return value;
}

int main(void)
{
        char directory[] = "/dev/shm/ttfx-real-raw.XXXXXX";
        assert(mkdtemp(directory));
        int dir = open(directory, O_RDONLY | O_DIRECTORY);
        assert(dir >= 0);
        ply_pixel_buffer_t *buffer = ply_pixel_buffer_new(1280, 720);
        assert(buffer && ply_pixel_buffer_get_device_scale(buffer) == 1);
        ply_rectangle_t full = {.width = 1280, .height = 720};
        ply_rectangle_t mark = {.x = 311, .y = 219, .width = 107, .height = 83};
        ply_pixel_buffer_fill_with_hex_color(buffer, &full, 0x0b0d10);
        ply_pixel_buffer_fill_with_hex_color(buffer, &mark, 0x123456);
        uint32_t *pixels = ply_pixel_buffer_get_argb32_data(buffer);
        assert(pixels && (pixels[219 * 1280 + 311] & 0xffffff) == 0x123456);
        uint64_t stamp = ttfx_boottime_ns();
        assert(ttfx_raw_publish(dir, pixels, 1280, 720, stamp));
        int fd = openat(dir, TTFX_RAW_NAME, O_RDONLY | O_NOFOLLOW);
        struct stat st;
        assert(fd >= 0 && fstat(fd, &st) == 0);
        assert(S_ISREG(st.st_mode) && st.st_nlink == 1 && st.st_uid == geteuid());
        assert((st.st_mode & 0777) == 0644 && st.st_size == 48 + 1280 * 720 * 4);
        uint8_t *raw = malloc((size_t)st.st_size);
        assert(raw && read(fd, raw, (size_t)st.st_size) == st.st_size);
        close(fd);
        assert(memcmp(raw, "OMBFRAW1", 8) == 0);
        assert(little(raw + 8, 4) == 1280 && little(raw + 12, 4) == 720);
        assert(little(raw + 16, 4) == 5120 && little(raw + 20, 4) == 0x34325258);
        assert(little(raw + 24, 8) == 1280 * 720 * 4);
        assert(little(raw + 32, 8) == stamp && little(raw + 40, 8) == 0);
        for (size_t i = 0; i < 1280 * 720; i++)
                assert(little(raw + 48 + i * 4, 4) == (pixels[i] & 0xffffff));
        free(raw);
        assert(!ttfx_raw_publish(dir, pixels, 4097, 720, stamp));
        assert(faccessat(dir, TTFX_RAW_NAME, F_OK, 0) != 0);
        assert(!ttfx_raw_publish(dir, pixels, 1280, 2161, stamp));
        assert(!ttfx_raw_publish(dir, pixels, 1280, 720, UINT64_MAX));
        assert(!ttfx_raw_publish(dir, pixels, 1280, 720, 0));
        assert(symlinkat("/no-such-target", dir, TTFX_RAW_TEMP) == 0);
        assert(!ttfx_raw_publish(dir, pixels, 1280, 720, stamp));
        assert(unlinkat(dir, TTFX_RAW_TEMP, 0) == 0);
        assert(ttfx_raw_publish(dir, pixels, 1280, 720, stamp));
        ttfx_handoff_clear(dir);
        assert(faccessat(dir, TTFX_RAW_NAME, F_OK, 0) != 0);
        ply_pixel_buffer_free(buffer);
        close(dir);
        assert(rmdir(directory) == 0);
        puts("real Plymouth pixelbuffer -> OMBFRAW1 full-frame readback: PASS (1280x720, every pixel)");
}
