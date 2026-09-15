#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "state.h"
#include <fcntl.h>
#include <inttypes.h>
#include <stdio.h>
#include <sys/stat.h>
#include <sys/vfs.h>
#include <linux/magic.h>
#include <unistd.h>

#include <stdlib.h>
#include <string.h>

#include <time.h>

uint64_t ttfx_boottime_ns(void)
{
        struct timespec now;
        if (clock_gettime(CLOCK_BOOTTIME, &now) != 0 || now.tv_sec < 0)
                return 0;
        return (uint64_t)now.tv_sec * UINT64_C(1000000000) + (uint64_t)now.tv_nsec;
}

void ttfx_raw_clear(int directory)
{
        if (directory >= 0)
                (void)unlinkat(directory, TTFX_RAW_NAME, 0);
}

static void put_little(uint8_t *out, uint64_t value, unsigned int bytes)
{
        for (unsigned int i = 0; i < bytes; i++)
                out[i] = (uint8_t)(value >> (i * 8U));
}

bool ttfx_raw_publish(int directory, const uint32_t *pixels, uint32_t width,
                      uint32_t height, uint64_t captured_ns)
{
        uint8_t header[48] = {0};
        uint8_t row[TTFX_RAW_MAX_WIDTH * 4U];
        int file = -1;
        bool created = false;
        uint64_t now = ttfx_boottime_ns();
        if (directory < 0)
                return false;
        if (!pixels || !width || !height || width > TTFX_RAW_MAX_WIDTH ||
            height > TTFX_RAW_MAX_HEIGHT || !captured_ns || captured_ns > now ||
            now - captured_ns > UINT64_C(30000000000))
                goto fail;
        memcpy(header, "OMBFRAW1", 8);
        put_little(header + 8, width, 4);
        put_little(header + 12, height, 4);
        put_little(header + 16, width * 4U, 4);
        put_little(header + 20, UINT32_C(0x34325258), 4);
        put_little(header + 24, (uint64_t)width * height * 4U, 8);
        put_little(header + 32, captured_ns, 8);
        file = openat(directory, TTFX_RAW_TEMP,
                      O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK, 0600);
        if (file < 0)
                goto fail;
        created = true;
        if (write(file, header, sizeof(header)) != (ssize_t)sizeof(header))
                goto fail;
        /* Packed native Plymouth ARGB words -> explicit little-endian BGRX.
         * At most 2160 writes, no retry/sync/sleep and only 16 KiB scratch. */
        for (uint32_t y = 0; y < height; y++) {
                for (uint32_t x = 0; x < width; x++)
                        put_little(row + x * 4U, pixels[(size_t)y * width + x] & 0xffffffU, 4);
                if (write(file, row, width * 4U) != (ssize_t)(width * 4U))
                        goto fail;
        }
        if (fchmod(file, 0644) != 0)
                goto fail;
        int closed = close(file);
        file = -1;
        if (closed != 0 || renameat(directory, TTFX_RAW_TEMP, directory, TTFX_RAW_NAME) != 0)
                goto fail;
        return true;
fail:
        if (file >= 0)
                (void)close(file);
        if (created)
                (void)unlinkat(directory, TTFX_RAW_TEMP, 0);
        ttfx_raw_clear(directory);
        return false;
}

void ttfx_handoff_clear(int directory)
{
        if (directory >= 0)
                (void)unlinkat(directory, TTFX_HANDOFF_NAME, 0);
        ttfx_raw_clear(directory);
}

void ttfx_handoff_close(int directory)
{
        if (directory >= 0)
                (void)close(directory);
}

int ttfx_handoff_open(void)
{
        struct stat st;
        struct statfs fs;
        if (geteuid() != 0)
                return -1;
        int directory = open("/run", O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK);
        if (directory < 0)
                return -1;
        if (fstat(directory, &st) != 0 || st.st_uid != 0 || (st.st_mode & 0022) != 0 ||
            fstatfs(directory, &fs) != 0 || fs.f_type != TMPFS_MAGIC) {
                close(directory);
                return -1;
        }
        ttfx_handoff_clear(directory);
        return directory;
}

bool ttfx_handoff_publish(int directory, const ttfx_handoff_t *h)
{
        char text[TTFX_HANDOFF_MAX_BYTES];
        size_t length = 0;
        int file = -1;
        bool created = false;
        if (directory < 0)
                return false;
        if (h == NULL || h->effect == NULL || h->phase.step > TTFX_HANDOFF_MAX_STEP ||
            h->width != 162U || h->height != 20U || h->fps != 240U || h->speed != 1U ||
            h->background > 0xffffffU || h->foreground > 0xffffffU)
                goto fail;
        while (length < 128U && h->effect[length] != '\0') {
                char c = h->effect[length];
                if (!((c >= 'a' && c <= 'z') ||
                    (length > 0U && ((c >= '0' && c <= '9') || c == '-'))))
                        goto fail;
                length++;
        }
        if (length == 0U || length == 128U)
                goto fail;
        int size = snprintf(text, sizeof(text),
                "version=1\neffect=%s\nseed=%" PRIu64 "\ncycle=%" PRIu64 "\nstep=%" PRIu64
                "\nwidth=%" PRIu32 "\nheight=%" PRIu32 "\nfps=%" PRIu32 "\nspeed=%" PRIu32
                "\nbackground=%06" PRIx32 "\nforeground=%06" PRIx32 "\ninput=embedded-logo-v2\n",
                h->effect, h->seed, h->phase.cycle, h->phase.step,
                h->width, h->height, h->fps, h->speed, h->background, h->foreground);
        if (size <= 0 || (size_t)size >= sizeof(text))
                goto fail;
        /* Fixed-size tmpfs I/O only: no disk sync, wait, sleep or retry loops.
         * O_EXCL prevents following symlinks/FIFOs and never truncates an existing leaf. */
        file = openat(directory, TTFX_HANDOFF_TEMP,
                      O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK, 0600);
        if (file < 0)
                goto fail;
        created = true;
        if (write(file, text, (size_t)size) != size || fchmod(file, 0644) != 0)
                goto fail;
        int closed = close(file);
        file = -1;
        if (closed != 0 || renameat(directory, TTFX_HANDOFF_TEMP, directory, TTFX_HANDOFF_NAME) != 0)
                goto fail;
        return true;
fail:
        if (file >= 0)
                (void)close(file);
        if (created)
                (void)unlinkat(directory, TTFX_HANDOFF_TEMP, 0);
        ttfx_handoff_clear(directory);
        return false;
}

struct ttfx_message {
        char *text;
        ttfx_message_t *next;
};

static unsigned int min_u(unsigned int left, unsigned int right)
{
        return left < right ? left : right;
}

ttfx_state_t ttfx_state_initial(void)
{
        ttfx_state_t state = {
                .prompt_mode = TTFX_PROMPT_NONE,
                .animation_enabled = true
        };
        return state;
}

static char *copy_string(const char *text)
{
        const char *source = text != NULL ? text : "";
        size_t size = strlen(source) + 1U;
        char *copy = malloc(size);

        if (copy != NULL)
                memcpy(copy, source, size);
        return copy;
}

void ttfx_state_destroy(ttfx_state_t *state)
{
        ttfx_message_t *message = state->messages;

        free(state->prompt);
        free(state->entry_text);
        free(state->failure_candidate_prompt);
        while (message != NULL) {
                ttfx_message_t *next = message->next;
                free(message->text);
                free(message);
                message = next;
        }
        state->prompt = NULL;
        state->entry_text = NULL;
        state->failure_candidate_prompt = NULL;
        state->message = NULL;
        state->messages = NULL;
        state->message_visible = false;
}

void ttfx_state_tick(ttfx_state_t *state)
{
        state->frame = (state->frame + 1U) % TTFX_FRAME_COUNT;
        if (state->failure_candidate_ready) {
                state->failure_candidate_tick++;
                if (state->failure_candidate_tick >= TTFX_FAILURE_CANDIDATE_TICKS) {
                        free(state->failure_candidate_prompt);
                        state->failure_candidate_prompt = NULL;
                        state->failure_candidate_ready = false;
                        state->failure_candidate_tick = 0U;
                        state->prompt_mode = TTFX_PROMPT_NONE;
                        state->bullets = 0;
                        free(state->prompt);
                        state->prompt = NULL;
                        free(state->entry_text);
                        state->entry_text = NULL;
                }
        }
        if (state->reaction_active) {
                state->reaction_tick++;
                if (state->reaction_tick >= TTFX_REACTION_TICKS)
                        state->reaction_active = false;
        }
}

void ttfx_state_disable_animation(ttfx_state_t *state)
{
        state->animation_enabled = false;
}

void ttfx_state_fail_rendering(ttfx_state_t *state)
{
        ttfx_state_disable_animation(state);
        state->rendering_failed = true;
}

void ttfx_state_set_normal(ttfx_state_t *state)
{
        free(state->failure_candidate_prompt);
        state->failure_candidate_prompt = NULL;
        state->failure_candidate_ready = false;
        state->failure_candidate_tick = 0U;
        state->reaction_active = false;
        state->reaction_tick = 0U;
        if (state->prompt_mode == TTFX_PROMPT_PASSWORD && state->bullets > 0 &&
            state->prompt != NULL && state->prompt[0] != '\0') {
                state->failure_candidate_ready = true;
                return;
        }
        ttfx_state_clear_prompt(state);
}

void ttfx_state_clear_prompt(ttfx_state_t *state)
{
        free(state->failure_candidate_prompt);
        state->failure_candidate_prompt = NULL;
        state->failure_candidate_ready = false;
        state->failure_candidate_tick = 0U;
        state->reaction_active = false;
        state->reaction_tick = 0U;
        state->prompt_mode = TTFX_PROMPT_NONE;
        state->bullets = 0;
        free(state->prompt);
        free(state->entry_text);
        state->prompt = NULL;
        state->entry_text = NULL;
}

bool ttfx_state_set_password(ttfx_state_t *state, const char *prompt, int bullets)
{
        char *prompt_copy = copy_string(prompt);
        int safe_bullets = bullets < 0 ? 0 : bullets;
        if (prompt_copy == NULL)
                return false;

        if (safe_bullets == 0 && state->failure_candidate_ready &&
            state->prompt != NULL && strcmp(state->prompt, prompt_copy) == 0) {
                state->reaction_active = true;
                state->reaction_tick = 0U;
        }
        free(state->failure_candidate_prompt);
        state->failure_candidate_prompt = NULL;
        state->failure_candidate_ready = false;
        state->failure_candidate_tick = 0U;

        free(state->prompt);
        free(state->entry_text);
        state->prompt = prompt_copy;
        state->entry_text = NULL;
        state->prompt_mode = TTFX_PROMPT_PASSWORD;
        state->bullets = safe_bullets;
        return true;
}

bool ttfx_state_password_pending(const ttfx_state_t *state)
{
        return state->failure_candidate_ready;
}

bool ttfx_state_reaction_active(const ttfx_state_t *state)
{
        return state->reaction_active;
}

int ttfx_state_reaction_x_offset(const ttfx_state_t *state)
{
        static const int offsets[] = {-8, 6, -5, 8, -4, 5, -2};

        if (!state->reaction_active)
                return 0;
        return offsets[(state->reaction_tick / TTFX_REACTION_OFFSET_TICKS) %
                       (sizeof(offsets) / sizeof(offsets[0]))];
}

bool ttfx_state_reaction_row_red(const ttfx_state_t *state, unsigned int row)
{
        (void)row;
        return state->reaction_active;
}

bool ttfx_state_set_question(ttfx_state_t *state, const char *prompt, const char *entry_text)
{
        char *prompt_copy = copy_string(prompt);
        char *entry_copy = copy_string(entry_text);
        if (prompt_copy == NULL || entry_copy == NULL) {
                free(prompt_copy);
                free(entry_copy);
                return false;
        }

        free(state->failure_candidate_prompt);
        state->failure_candidate_prompt = NULL;
        state->failure_candidate_ready = false;
        state->failure_candidate_tick = 0U;
        state->reaction_active = false;
        state->reaction_tick = 0U;
        free(state->prompt);
        free(state->entry_text);
        state->prompt = prompt_copy;
        state->entry_text = entry_copy;
        state->prompt_mode = TTFX_PROMPT_QUESTION;
        state->bullets = 0;
        return true;
}

bool ttfx_state_set_message(ttfx_state_t *state, const char *message)
{
        char *message_copy = copy_string(message);
        ttfx_message_t *new_message;

        if (message_copy == NULL)
                return false;
        new_message = malloc(sizeof(*new_message));
        if (new_message == NULL) {
                free(message_copy);
                return false;
        }
        new_message->text = message_copy;
        new_message->next = state->messages;
        state->messages = new_message;
        state->message = new_message->text;
        state->message_visible = true;
        return true;
}

bool ttfx_state_hide_message(ttfx_state_t *state, const char *message)
{
        ttfx_message_t **link = &state->messages;

        if (message == NULL)
                return false;
        while (*link != NULL) {
                ttfx_message_t *candidate = *link;
                if (strcmp(candidate->text, message) == 0) {
                        *link = candidate->next;
                        free(candidate->text);
                        free(candidate);
                        state->message = state->messages != NULL ? state->messages->text : NULL;
                        state->message_visible = state->message != NULL;
                        return true;
                }
                link = &candidate->next;
        }
        return false;
}

ttfx_view_state_t ttfx_view_state_initial(void)
{
        ttfx_view_state_t state = {
                .prompt_label_available = true,
                .message_label_available = true
        };
        return state;
}

bool ttfx_view_show_prompt(ttfx_view_state_t *state)
{
        bool was_hidden = !state->prompt_visible;
        state->prompt_visible = true;
        return was_hidden;
}

bool ttfx_view_hide_prompt(ttfx_view_state_t *state)
{
        bool was_visible = state->prompt_visible;
        state->prompt_visible = false;
        return was_visible;
}

bool ttfx_view_show_message(ttfx_view_state_t *state)
{
        bool was_hidden = !state->message_visible;
        state->message_visible = true;
        return was_hidden;
}

bool ttfx_view_hide_message(ttfx_view_state_t *state)
{
        bool was_visible = state->message_visible;
        state->message_visible = false;
        return was_visible;
}

bool ttfx_view_prompt_can_show(const ttfx_view_state_t *state)
{
        return state->prompt_label_available;
}

bool ttfx_view_message_can_show(const ttfx_view_state_t *state)
{
        return state->message_label_available;
}

void ttfx_view_degrade_prompt(ttfx_state_t *plugin_state, ttfx_view_state_t *view_state)
{
        ttfx_state_disable_animation(plugin_state);
        view_state->prompt_label_available = false;
}

void ttfx_view_degrade_message(ttfx_state_t *plugin_state, ttfx_view_state_t *view_state)
{
        ttfx_state_disable_animation(plugin_state);
        view_state->message_label_available = false;
        view_state->message_visible = false;
}

ttfx_loop_state_t ttfx_loop_state_initial(void)
{
        ttfx_loop_state_t state = {false};
        return state;
}

void ttfx_loop_watch_attached(ttfx_loop_state_t *state)
{
        state->exit_watch_attached = true;
}

bool ttfx_loop_detach_watch(ttfx_loop_state_t *state)
{
        bool was_attached = state->exit_watch_attached;
        state->exit_watch_attached = false;
        return was_attached;
}

ttfx_load_state_t ttfx_load_state_initial(void)
{
        ttfx_load_state_t state = {true};
        return state;
}

void ttfx_load_state_record_view(ttfx_load_state_t *state, bool loaded)
{
        state->required_widgets_ready = state->required_widgets_ready && loaded;
}

bool ttfx_load_state_can_show(const ttfx_load_state_t *state)
{
        return state->required_widgets_ready;
}

bool ttfx_dynamic_view_should_keep(bool loaded)
{
        return loaded;
}

/* Official wordmark lattice after exact 2x2 subdivision: 51 units wide per
 * 100-unit terminal row. 162 columns at height 10 still span 826px. */
unsigned int ttfx_geometry_x_edge(const ttfx_geometry_t *geometry, unsigned int column)
{
        return (unsigned int)((uint64_t)column * geometry->cell_height * 51U / 100U);
}

ttfx_geometry_t ttfx_geometry_for_view(unsigned int width, unsigned int height)
{
        ttfx_geometry_t geometry;

        geometry.columns = min_u(TTFX_GRID_COLUMNS, width);
        geometry.rows = min_u(TTFX_GRID_ROWS, height);
        geometry.cell_height = geometry.rows > 0U && geometry.columns > 0U
                                       ? min_u(10U, height / geometry.rows)
                                       : 0U;
        if (geometry.columns > 0U) {
                unsigned int width_limited_height = (unsigned int)
                        ((uint64_t)width * 100U / ((uint64_t)geometry.columns * 51U));
                geometry.cell_height = min_u(geometry.cell_height, width_limited_height);
        }
        /* Informational minimum width; rendering must use cumulative edges. */
        geometry.cell_width = ttfx_geometry_x_edge(&geometry, 1U);
        geometry.grid_width = ttfx_geometry_x_edge(&geometry, geometry.columns);
        geometry.grid_height = geometry.cell_height * geometry.rows;
        geometry.grid_x = (width - geometry.grid_width) / 2U;
        geometry.grid_y = (height - geometry.grid_height) * 9U / 20U;
        return geometry;
}


