#define _GNU_SOURCE
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
static int fail_write, fail_rename;
static ssize_t test_write(int fd, const void *data, size_t size)
{
        if (fail_write) return size > 0 ? (ssize_t)size - 1 : -1;
        return write(fd, data, size);
}
static int test_renameat(int from, const char *a, int to, const char *b)
{
        if (fail_rename) return -1;
        return renameat(from, a, to, b);
}
#define write test_write
#define renameat test_renameat
#include "../state.c"
#undef write
#undef renameat

int main(void)
{
        char directory[] = "/dev/shm/ttfx-handoff-test.XXXXXX";
        assert(mkdtemp(directory) != NULL);
        int dir = open(directory, O_DIRECTORY | O_RDONLY | O_CLOEXEC);
        assert(dir >= 0);
        ttfx_handoff_t handoff = {
                .effect = "vhstape", .seed = UINT64_MAX,
                .phase = {.step = 17, .cycle = 2},
                .width = 162, .height = 20, .fps = 240, .speed = 1,
                .background = 0x0b0d10, .foreground = 0xf4f4f5
        };
        assert(ttfx_handoff_publish(dir, &handoff));
        int file = openat(dir, TTFX_HANDOFF_NAME, O_RDONLY | O_NOFOLLOW);
        assert(file >= 0);
        struct stat st;
        assert(fstat(file, &st) == 0 && st.st_uid == geteuid());
        assert((st.st_mode & 0777) == 0644 && st.st_size < TTFX_HANDOFF_MAX_BYTES);
        char text[TTFX_HANDOFF_MAX_BYTES] = {0};
        assert(read(file, text, sizeof(text) - 1) == st.st_size);
        close(file);
        assert(strstr(text, "version=1\neffect=vhstape\nseed=18446744073709551615\ncycle=2\nstep=17\n") != NULL);
        assert(strstr(text, "fps=240\nspeed=1\n") != NULL);
        assert(strstr(text, "width=162\nheight=20\n") != NULL);
        assert(strstr(text, "input=embedded-logo-v2\n") != NULL);
        /* A colliding temporary leaf (including symlink) cannot be followed.
         * Publication fails closed, removing stale final state, not its target. */
        assert(symlinkat("/does-not-exist", dir, TTFX_HANDOFF_TEMP) == 0);
        assert(!ttfx_handoff_publish(dir, &handoff));
        assert(faccessat(dir, TTFX_HANDOFF_NAME, F_OK, 0) != 0);
        unlinkat(dir, TTFX_HANDOFF_TEMP, 0);
        assert(ttfx_handoff_publish(dir, &handoff));
        fail_write = 1;
        assert(!ttfx_handoff_publish(dir, &handoff));
        assert(faccessat(dir, TTFX_HANDOFF_NAME, F_OK, 0) != 0);
        assert(faccessat(dir, TTFX_HANDOFF_TEMP, F_OK, 0) != 0);
        fail_write = 0;
        fail_rename = 1;
        assert(!ttfx_handoff_publish(dir, &handoff));
        assert(faccessat(dir, TTFX_HANDOFF_TEMP, F_OK, 0) != 0);
        fail_rename = 0;
        assert(symlinkat("/does-not-exist", dir, TTFX_HANDOFF_NAME) == 0);
        assert(ttfx_handoff_publish(dir, &handoff));
        assert(fstatat(dir, TTFX_HANDOFF_NAME, &st, AT_SYMLINK_NOFOLLOW) == 0);
        assert(S_ISREG(st.st_mode));
        handoff.phase.step = TTFX_HANDOFF_MAX_STEP + 1U;
        assert(!ttfx_handoff_publish(dir, &handoff));
        assert(faccessat(dir, TTFX_HANDOFF_NAME, F_OK, 0) != 0);
        handoff.phase.step = 0;
        handoff.effect = "bad\nstep=9";
        assert(!ttfx_handoff_publish(dir, &handoff));
        assert(!ttfx_handoff_publish(-1, &handoff));
        uint32_t pixels[] = {0xff123456U, 0xffabcdefU};
        uint64_t stamp = ttfx_boottime_ns();
        assert(ttfx_raw_publish(dir, pixels, 2, 1, stamp));
        fail_write = 1;
        assert(!ttfx_raw_publish(dir, pixels, 2, 1, stamp));
        assert(faccessat(dir, TTFX_RAW_NAME, F_OK, 0) != 0);
        assert(faccessat(dir, TTFX_RAW_TEMP, F_OK, 0) != 0);
        fail_write = 0;
        fail_rename = 1;
        assert(!ttfx_raw_publish(dir, pixels, 2, 1, stamp));
        assert(faccessat(dir, TTFX_RAW_TEMP, F_OK, 0) != 0);
        fail_rename = 0;
        assert(symlinkat("/does-not-exist", dir, TTFX_RAW_NAME) == 0);
        assert(ttfx_raw_publish(dir, pixels, 2, 1, stamp));
        assert(fstatat(dir, TTFX_RAW_NAME, &st, AT_SYMLINK_NOFOLLOW) == 0 && S_ISREG(st.st_mode));
        ttfx_handoff_clear(dir);
        assert(faccessat(dir, TTFX_RAW_NAME, F_OK, 0) != 0);
        close(dir);
        assert(rmdir(directory) == 0);
        puts("handoff publication tests: PASS");
        return 0;
}
