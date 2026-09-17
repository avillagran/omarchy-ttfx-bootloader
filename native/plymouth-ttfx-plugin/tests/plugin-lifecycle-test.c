#define _GNU_SOURCE
#include <fcntl.h>
#include <unistd.h>
#include <assert.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>

#include "stubs/ply-stubs.h"
#include "ttfx_plymouth.h"

struct ply_event_loop {
        unsigned int exit_watches;
        unsigned int exit_stops;
        unsigned int timeout_watches;
        unsigned int timeout_stops;
        ply_event_loop_timeout_handler_t timeout_handler;
        void *timeout_data;
};

struct ply_pixel_display {
        unsigned long width;
        unsigned long height;
        ply_pixel_display_draw_handler_t draw_handler;
        void *draw_data;
};

struct ply_pixel_buffer { unsigned long width, height; uint32_t *pixels; int scale, rotation; };
int ply_pixel_buffer_get_device_scale(ply_pixel_buffer_t *b) { return b->scale; }
ply_pixel_buffer_rotation_t ply_pixel_buffer_get_device_rotation(ply_pixel_buffer_t *b) { return (ply_pixel_buffer_rotation_t)b->rotation; }
unsigned long ply_pixel_buffer_get_width(ply_pixel_buffer_t *b) { return b->width; }
unsigned long ply_pixel_buffer_get_height(ply_pixel_buffer_t *b) { return b->height; }
uint32_t *ply_pixel_buffer_get_argb32_data(ply_pixel_buffer_t *b) { return b->pixels; }
ply_pixel_buffer_t *ply_pixel_buffer_new(unsigned long width, unsigned long height)
{
        ply_pixel_buffer_t *buffer = calloc(1U, sizeof(*buffer));
        assert(buffer != NULL);
        buffer->width = width;
        buffer->height = height;
        buffer->pixels = calloc(width * height, sizeof(*buffer->pixels));
        assert(buffer->pixels != NULL);
        return buffer;
}
void ply_pixel_buffer_free(ply_pixel_buffer_t *buffer)
{
        if (buffer == NULL)
                return;
        free(buffer->pixels);
        free(buffer);
}
struct ply_entry { int unused; };
struct ply_label { bool backend_loaded; unsigned int id; char text[128]; };
struct _ply_image { long width, height; ply_pixel_buffer_t buffer; };
struct ply_key_file {
        const char *image_dir;
        const char *mode;
        const char *enabled;
        const char *effect;
        const char *seed;
        const char *background;
        const char *text;
        const char *message;
        const char *playback;
};
struct ply_buffer { int unused; };

static unsigned int entry_allocations;
static unsigned int entry_frees;
static unsigned int entry_loads;
static unsigned int entry_shows;
static unsigned int entry_hides;
static long last_entry_show_x;
static long last_entry_show_y;
static long entry_show_x[32];
static long entry_show_y[32];
static int last_bullet_count;
static unsigned int entry_draws;
static unsigned int label_allocations;
static unsigned int label_frees;
static unsigned int label_shows;
static unsigned int label_hides;
static unsigned int draw_handler_sets;
static unsigned int draw_handler_clears;
static unsigned int draw_requests;
static unsigned int engine_creates;
static unsigned int engine_steps;
static unsigned int engine_cells_calls;
static unsigned int engine_frees;
static unsigned int engine_resets;
static unsigned int label_color_sets;
static unsigned int password_prompt_label_shows;
static unsigned int prompt_label_draws;
static unsigned int message_label_shows;
static unsigned int message_label_draws;
static unsigned int image_allocations;
static unsigned int image_frees;
static unsigned int image_loads;
static unsigned int image_resizes;
static unsigned int image_composites;
static unsigned int opacity_composites;
static float opacity_samples[256];
static unsigned int success_green_fills;
static double success_green_opacity;
static long resized_image_width;
static long resized_image_height;
static int image_composite_x;
static ply_pixel_buffer_t *last_fill_source;
static int image_composite_y;
static bool mock_image_load_success;
static unsigned int logo_color_fills;
static unsigned int glitch_red_fills;
static int32_t mock_step_status;
static unsigned int mock_fail_on_step;
static unsigned int mock_loop_on_step;
static int32_t mock_cells_status;
static int32_t mock_create_status;
static bool mock_snapshot_ready;
static unsigned int red_fills;
static unsigned int green_fills;
static unsigned int blue_fills;
static uint32_t primary_fill_order[8];
static size_t primary_fill_count;
static bool capture_geometry;
static ply_rectangle_t captured_rects[64];
static size_t captured_count;
static unsigned int clip_pushes;
static ply_rectangle_t last_clip;
static char created_effect[128];
static uint64_t created_seed;
static uint32_t created_background_rgb;
static uint64_t mock_fade_now_ns = UINT64_C(1000000000);
static uint64_t mock_fade_clock_ns(void) { return mock_fade_now_ns; }

#define SECRET_ENTRY_PIXEL UINT32_C(0xffdec0de)
#define SECRET_LABEL_PIXEL UINT32_C(0xff51a7e1)

struct TtfxEngine { int unused; };
static struct TtfxEngine mock_engine;
static TtfxCell mock_cells[162U * 20U];

/* Never touch the host /run, even if this test is launched as root. */
#define ttfx_handoff_open test_handoff_open
#include "../plugin.c"
#undef ttfx_handoff_open
int test_handoff_open(void) { return -1; }

char *ply_key_file_get_value(ply_key_file_t *key_file, const char *section, const char *key)
{
        const char *value = NULL;
        char *copy;

        if (key_file == NULL || strcmp(section, "ttfx") != 0)
                return NULL;
        if (strcmp(key, "ImageDir") == 0) value = key_file->image_dir;
        else if (strcmp(key, "Mode") == 0) value = key_file->mode;
        else if (strcmp(key, "Enabled") == 0) value = key_file->enabled;
        else if (strcmp(key, "Effect") == 0) value = key_file->effect;
        else if (strcmp(key, "Seed") == 0) value = key_file->seed;
        else if (strcmp(key, "BackgroundColor") == 0) value = key_file->background;
        else if (strcmp(key, "TextColor") == 0) value = key_file->text;
        else if (strcmp(key, "MessageColor") == 0) value = key_file->message;
        else if (strcmp(key, "PlaybackMode") == 0) value = key_file->playback;
        if (value == NULL)
                return NULL;
        copy = malloc(strlen(value) + 1U);
        assert(copy != NULL);
        strcpy(copy, value);
        return copy;
}

void ply_event_loop_watch_for_exit(ply_event_loop_t *loop,
                                   ply_event_loop_exit_handler_t handler,
                                   void *data)
{
        (void)handler;
        (void)data;
        loop->exit_watches++;
}

void ply_event_loop_stop_watching_for_exit(ply_event_loop_t *loop,
                                           ply_event_loop_exit_handler_t handler,
                                           void *data)
{
        (void)handler;
        (void)data;
        loop->exit_stops++;
}

void ply_event_loop_watch_for_timeout(ply_event_loop_t *loop,
                                      double seconds,
                                      ply_event_loop_timeout_handler_t handler,
                                      void *data)
{
        assert(seconds == 1.0 / 240.0);
        loop->timeout_handler = handler;
        loop->timeout_data = data;
        loop->timeout_watches++;
}

void ply_event_loop_stop_watching_for_timeout(ply_event_loop_t *loop,
                                              ply_event_loop_timeout_handler_t handler,
                                              void *data)
{
        (void)handler;
        (void)data;
        loop->timeout_stops++;
}

unsigned long ply_pixel_display_get_width(ply_pixel_display_t *display) { return display->width; }
unsigned long ply_pixel_display_get_height(ply_pixel_display_t *display) { return display->height; }

void ply_pixel_display_set_draw_handler(ply_pixel_display_t *display,
                                        ply_pixel_display_draw_handler_t handler,
                                        void *data)
{
        display->draw_handler = handler;
        display->draw_data = data;
        if (handler == NULL)
                draw_handler_clears++;
        else
                draw_handler_sets++;
}

void ply_pixel_display_draw_area(ply_pixel_display_t *display, int x, int y, int width, int height)
{
        (void)display;
        (void)x;
        (void)y;
        (void)width;
        (void)height;
        draw_requests++;
}

void ply_pixel_buffer_push_clip_area(ply_pixel_buffer_t *buffer, ply_rectangle_t *area)
{
        (void)buffer;
        clip_pushes++;
        last_clip = *area;
}
void ply_pixel_buffer_pop_clip_area(ply_pixel_buffer_t *buffer) { (void)buffer; }
void ply_pixel_buffer_fill_with_hex_color(ply_pixel_buffer_t *buffer,
                                          ply_rectangle_t *area,
                                          uint32_t color)
{
        if (buffer != NULL && buffer->pixels != NULL) {
                long left = area->x < 0 ? 0 : area->x;
                long top = area->y < 0 ? 0 : area->y;
                unsigned long right = area->x < 0 && (unsigned long)-area->x >= area->width
                                              ? 0U
                                              : (unsigned long)(area->x + (long)area->width);
                unsigned long bottom = area->y < 0 && (unsigned long)-area->y >= area->height
                                               ? 0U
                                               : (unsigned long)(area->y + (long)area->height);
                if (right > buffer->width) right = buffer->width;
                if (bottom > buffer->height) bottom = buffer->height;
                for (unsigned long row = (unsigned long)top; row < bottom; row++)
                        for (unsigned long column = (unsigned long)left; column < right; column++)
                                buffer->pixels[row * buffer->width + column] = UINT32_C(0xff000000) | color;
        }
        if (capture_geometry) {
                assert(captured_count < sizeof(captured_rects) / sizeof(captured_rects[0]));
                captured_rects[captured_count++] = *area;
        }
        if (color == 0xff0000U)
                red_fills++;
        if (color == 0x00ff00U)
                green_fills++;
        if (color == 0x0000ffU)
                blue_fills++;
        if (color == LOGO_COLOR)
                logo_color_fills++;
        if (color == UINT32_C(0xff4057))
                glitch_red_fills++;
        if ((color == 0xff0000U || color == 0x00ff00U || color == 0x0000ffU) &&
            primary_fill_count < sizeof(primary_fill_order) / sizeof(primary_fill_order[0]))
                primary_fill_order[primary_fill_count++] = color;
}

void ply_pixel_buffer_fill_with_hex_color_at_opacity(ply_pixel_buffer_t *buffer,
                                                     ply_rectangle_t *area,
                                                     uint32_t color,
                                                     double opacity)
{
        (void)buffer;
        (void)area;
        if (color == UINT32_C(0x4ade80)) {
                success_green_fills++;
                success_green_opacity = opacity;
        }
}

void ply_pixel_buffer_fill_with_buffer(ply_pixel_buffer_t *canvas,
                                       ply_pixel_buffer_t *source,
                                       int x,
                                       int y)
{
        (void)canvas;
        assert(source != NULL);
        last_fill_source = source;
        image_composites++;
        image_composite_x = x;
        image_composite_y = y;
}

void ply_pixel_buffer_fill_with_buffer_at_opacity(ply_pixel_buffer_t *canvas,
                                                  ply_pixel_buffer_t *source,
                                                  int x, int y, float opacity)
{
        (void)canvas;
        assert(source != NULL);
        assert(opacity_composites < sizeof(opacity_samples) / sizeof(opacity_samples[0]));
        opacity_samples[opacity_composites++] = opacity;
        image_composites++;
        image_composite_x = x;
        image_composite_y = y;
}

ply_image_t *ply_image_new(const char *filename)
{
        const char *name = strrchr(filename, '/');
        name = name != NULL ? name + 1 : filename;
        ply_image_t *image = calloc(1U, sizeof(*image));
        assert(image != NULL);
        if (strcmp(name, "lock.png") == 0) {
                image->width = 84L;
                image->height = 96L;
        } else if (strcmp(name, "entry.png") == 0) {
                image->width = 286L;
                image->height = 48L;
        } else if (strcmp(name, "progress_box.png") == 0 ||
                   strcmp(name, "progress_bar.png") == 0) {
                image->width = 300L;
                image->height = 10L;
        } else {
                assert(strcmp(name, "bullet.png") == 0);
                image->width = 14L;
                image->height = 14L;
        }
        image->buffer.width = (unsigned long)image->width;
        image->buffer.height = (unsigned long)image->height;
        image->buffer.pixels = calloc((unsigned long)(image->width * image->height),
                                      sizeof(*image->buffer.pixels));
        assert(image->buffer.pixels != NULL);
        image_allocations++;
        return image;
}

void ply_image_free(ply_image_t *image)
{
        image_frees++;
        free(image->buffer.pixels);
        free(image);
}

bool ply_image_load(ply_image_t *image)
{
        assert(image != NULL);
        image_loads++;
        return mock_image_load_success;
}

long ply_image_get_width(ply_image_t *image) { return image->width; }
long ply_image_get_height(ply_image_t *image) { return image->height; }

ply_image_t *ply_image_resize(ply_image_t *image, long width, long height)
{
        assert(image != NULL);
        ply_image_t *resized = calloc(1U, sizeof(*resized));
        assert(resized != NULL);
        resized->width = width;
        resized->height = height;
        resized->buffer.width = (unsigned long)width;
        resized->buffer.height = (unsigned long)height;
        resized->buffer.pixels = calloc((unsigned long)(width * height),
                                        sizeof(*resized->buffer.pixels));
        assert(resized->buffer.pixels != NULL);
        resized_image_width = width;
        resized_image_height = height;
        image_allocations++;
        image_resizes++;
        return resized;
}

ply_pixel_buffer_t *ply_image_get_buffer(ply_image_t *image) { return &image->buffer; }

ply_entry_t *ply_entry_new(const char *image_dir)
{
        (void)image_dir;
        entry_allocations++;
        return calloc(1U, sizeof(ply_entry_t));
}
void ply_entry_free(ply_entry_t *entry)
{
        entry_frees++;
        free(entry);
}
bool ply_entry_load(ply_entry_t *entry)
{
        (void)entry;
        entry_loads++;
        return true;
}
long ply_entry_get_width(ply_entry_t *entry) { (void)entry; return 286L; }
long ply_entry_get_height(ply_entry_t *entry) { (void)entry; return 48L; }
void ply_entry_set_text(ply_entry_t *entry, const char *text) { (void)entry; (void)text; }
void ply_entry_set_bullet_count(ply_entry_t *entry, int bullets)
{
        (void)entry;
        last_bullet_count = bullets;
}
void ply_entry_show(ply_entry_t *entry, ply_event_loop_t *loop, ply_pixel_display_t *display,
                    long x, long y)
{
        (void)entry; (void)loop; (void)display;
        last_entry_show_x = x;
        last_entry_show_y = y;
        assert(entry_shows < sizeof(entry_show_x) / sizeof(entry_show_x[0]));
        entry_show_x[entry_shows] = x;
        entry_show_y[entry_shows] = y;
        entry_shows++;
}
void ply_entry_hide(ply_entry_t *entry) { (void)entry; entry_hides++; }
void ply_entry_draw_area(ply_entry_t *entry, ply_pixel_buffer_t *buffer, int x, int y,
                         unsigned long width, unsigned long height)
{
        (void)entry; (void)x; (void)y; (void)width; (void)height;
        if (buffer != NULL && buffer->pixels != NULL && buffer->width > 0U && buffer->height > 0U)
                buffer->pixels[0] = SECRET_ENTRY_PIXEL;
        entry_draws++;
}

ply_label_t *ply_label_new(void)
{
        label_allocations++;
        ply_label_t *label = calloc(1U, sizeof(*label));
        if (label != NULL)
                label->id = label_allocations;
        return label;
}
void ply_label_free(ply_label_t *label)
{
        label_frees++;
        free(label);
}
void ply_label_set_font(ply_label_t *label, const char *font) { (void)label; (void)font; }
void ply_label_set_hex_color(ply_label_t *label, uint32_t color)
{
        assert(label->backend_loaded);
        assert(color == PROMPT_LABEL_COLOR || color == MESSAGE_LABEL_COLOR ||
               color == 0x00ff00ffU || color == 0x0000ffffU);
        label_color_sets++;
}
void ply_label_set_text(ply_label_t *label, const char *text)
{
        assert(strlen(text) < sizeof(label->text));
        strcpy(label->text, text);
}
void ply_label_set_alignment(ply_label_t *label, int alignment) { (void)label; (void)alignment; }
void ply_label_set_width(ply_label_t *label, long width) { (void)label; (void)width; }
bool ply_label_show(ply_label_t *label, ply_pixel_display_t *display, long x, long y)
{
        (void)display; (void)x; (void)y;
        label->backend_loaded = true;
        label_shows++;
        if ((label->id % 2U) == 1U && label->text[0] != '\0')
                password_prompt_label_shows++;
        if ((label->id % 2U) == 0U && label->text[0] != '\0')
                message_label_shows++;
        return true;
}
void ply_label_hide(ply_label_t *label) { (void)label; label_hides++; }
void ply_label_draw_area(ply_label_t *label, ply_pixel_buffer_t *buffer, int x, int y,
                         unsigned long width, unsigned long height)
{
        (void)x; (void)y; (void)width; (void)height;
        if (buffer != NULL && buffer->pixels != NULL && buffer->width * buffer->height > 1U)
                buffer->pixels[1] = SECRET_LABEL_PIXEL;
        if ((label->id % 2U) == 1U)
                prompt_label_draws++;
        else
                message_label_draws++;
}

int32_t ttfx_engine_create_with_background(const char *effect, uint64_t seed,
                                           const uint8_t *input, size_t input_len,
                                           uint32_t width, uint32_t height, uint32_t fps,
                                           uint32_t background_rgb,
                                           TtfxEngine **out_engine)
{
        assert(strlen(effect) < sizeof(created_effect));
        strcpy(created_effect, effect);
        created_seed = seed;
        created_background_rgb = background_rgb;
        assert(input != NULL && input_len > 0U);
        assert(width == 162U && height == 20U && fps == 240U);
        engine_creates++;
        if (mock_create_status != TTFX_STATUS_OK) {
                *out_engine = NULL;
                return mock_create_status;
        }
        mock_cells[0].codepoint = UINT32_C(0x4f);
        mock_snapshot_ready = true;
        *out_engine = &mock_engine;
        return TTFX_STATUS_OK;
}

int32_t ttfx_engine_step(TtfxEngine *engine, uint8_t *out_looped)
{
        assert(engine == &mock_engine);
        engine_steps++;
        if (mock_fail_on_step == engine_steps)
                return TTFX_STATUS_ENGINE_ERROR;
        if (mock_step_status != TTFX_STATUS_OK)
                return mock_step_status;
        *out_looped = engine_steps == mock_loop_on_step;
        return TTFX_STATUS_OK;
}

int32_t ttfx_engine_cells(const TtfxEngine *engine, const TtfxCell **out_cells,
                          size_t *out_count, uint32_t *out_width, uint32_t *out_height)
{
        assert(engine == &mock_engine);
        assert(mock_snapshot_ready);
        engine_cells_calls++;
        if (mock_cells_status != TTFX_STATUS_OK)
                return mock_cells_status;
        *out_cells = mock_cells;
        *out_count = 162U * 20U;
        *out_width = 162U;
        *out_height = 20U;
        return TTFX_STATUS_OK;
}

int32_t ttfx_engine_reset(TtfxEngine *engine)
{
        assert(engine == &mock_engine);
        engine_resets++;
        return TTFX_STATUS_OK;
}

int32_t ttfx_engine_draw_logo(TtfxEngine *engine, uint32_t mode,
                              uint8_t *out_looped, uint8_t *out_finished)
{
        assert(engine == &mock_engine);
        engine_steps++;
        if (mock_fail_on_step == engine_steps)
                return TTFX_STATUS_ENGINE_ERROR;
        if (mock_step_status != TTFX_STATUS_OK)
                return mock_step_status;
        if (out_looped != NULL)
                *out_looped = 0U;
        if (out_finished != NULL)
                *out_finished = 0U;
        if (engine_steps == mock_loop_on_step) {
                if (mode == TTFX_LOGO_MODE_LOOP) {
                        if (out_looped != NULL)
                                *out_looped = 1U;
                } else if (out_finished != NULL) {
                        *out_finished = 1U;
                }
        }
        return TTFX_STATUS_OK;
}
void ttfx_engine_free(TtfxEngine *engine) { assert(engine == &mock_engine); engine_frees++; }

static void reset_counters(void)
{
        entry_allocations = 0U;
        entry_frees = 0U;
        entry_loads = 0U;
        entry_shows = 0U;
        entry_hides = 0U;
        last_entry_show_x = -9999L;
        last_entry_show_y = -9999L;
        memset(entry_show_x, 0, sizeof(entry_show_x));
        memset(entry_show_y, 0, sizeof(entry_show_y));
        last_bullet_count = -1;
        entry_draws = 0U;
        label_allocations = 0U;
        label_frees = 0U;
        label_shows = 0U;
        label_hides = 0U;
        draw_handler_sets = 0U;
        draw_handler_clears = 0U;
        draw_requests = 0U;
        engine_creates = 0U;
        engine_steps = 0U;
        engine_cells_calls = 0U;
        engine_frees = 0U;
        engine_resets = 0U;
        label_color_sets = 0U;
        password_prompt_label_shows = 0U;
        prompt_label_draws = 0U;
        message_label_shows = 0U;
        message_label_draws = 0U;
        image_allocations = 0U;
        image_frees = 0U;
        image_loads = 0U;
        image_resizes = 0U;
        image_composites = 0U;
        clip_pushes = 0U;
        memset(&last_clip, 0, sizeof(last_clip));
        opacity_composites = 0U;
        memset(opacity_samples, 0, sizeof(opacity_samples));
        success_green_fills = 0U;
        success_green_opacity = 0.0;
        resized_image_width = 0L;
        resized_image_height = 0L;
        image_composite_x = 0;
        image_composite_y = 0;
        mock_image_load_success = true;
        logo_color_fills = 0U;
        glitch_red_fills = 0U;
        mock_step_status = TTFX_STATUS_OK;
        mock_fail_on_step = 0U;
        mock_loop_on_step = 0U;
        mock_cells_status = TTFX_STATUS_OK;
        mock_create_status = TTFX_STATUS_OK;
        mock_snapshot_ready = false;
        red_fills = 0U;
        green_fills = 0U;
        blue_fills = 0U;
        primary_fill_count = 0U;
        memset(primary_fill_order, 0, sizeof(primary_fill_order));
        memset(mock_cells, 0, sizeof(mock_cells));
        created_effect[0] = '\0';
        created_seed = 0U;
        created_background_rgb = 0U;
        mock_fade_now_ns = UINT64_C(1000000000);
}

static void test_pending_validation_keeps_password_entry_visible_until_result(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display_password(plugin, "Password", 4);
        const unsigned int shows_before_normal = entry_shows;
        const unsigned int hides_before_normal = entry_hides;
        const unsigned int draws_before_normal = entry_draws;
        const unsigned int composites_before_normal = image_composites;
        const long entry_y = last_entry_show_y;

        display_normal(plugin);
        assert(ttfx_state_password_pending(&plugin->state));
        assert(plugin->state.prompt_mode == TTFX_PROMPT_PASSWORD);
        assert(plugin->state.bullets == 4);
        assert(plugin->views->state.prompt_visible);
        assert(!ttfx_state_reaction_active(&plugin->state));
        assert(entry_shows == shows_before_normal);
        assert(entry_hides == hides_before_normal);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(entry_draws == draws_before_normal);
        assert(image_composites == composites_before_normal + 2U);
        assert(glitch_red_fills == 0U);

        display_password(plugin, "Password", 0);
        assert(ttfx_state_reaction_active(&plugin->state));
        assert(plugin->views->state.prompt_visible);
        assert(entry_hides == hides_before_normal + 1U);
        assert(entry_shows == shows_before_normal + 1U);
        assert(last_entry_show_y == entry_y);
        assert(last_entry_show_x == (1280L - 286L) / 2L +
                                    ttfx_state_reaction_x_offset(&plugin->state));
        destroy_plugin(plugin);
}

static void test_original_progress_bar_replaces_pending_input_and_is_monotonic(void)
{
        ply_key_file_t config = {
                .image_dir = "/trusted/omarchy", .mode = "fixed", .enabled = "true",
                .effect = "decrypt", .seed = "7"
        };
        ply_event_loop_t loop = {0};
        ply_trigger_t trigger = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(&config);
        assert(plugin != NULL);
        plugin->clock_ns = mock_fade_clock_ns;
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, PLY_BOOT_SPLASH_MODE_BOOT_UP));
        display_password(plugin, "Password", 4);
        display_normal(plugin);
        assert(ttfx_state_password_pending(&plugin->state));
        assert(plugin->progress_visible);
        assert(plugin->progress_fraction == 0.0);

        unsigned int draws = entry_draws;
        unsigned int composites = image_composites;
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(entry_draws == draws);
        assert(image_composites == composites + 2U);

        for (unsigned int tick = 0U; tick <= TTFX_FAILURE_CANDIDATE_TICKS; tick++)
                loop.timeout_handler(loop.timeout_data, &loop);
        assert(ttfx_state_password_pending(&plugin->state));
        assert(plugin->progress_visible);

        mock_fade_now_ns += UINT64_C(15000000000);
        loop.timeout_handler(loop.timeout_data, &loop);
        assert(plugin->progress_fraction == 0.70);
        uint64_t progress_started_ns = plugin->progress_started_ns;
        double progress_fraction = plugin->progress_fraction;
        hide_splash_screen(plugin, &loop);
        assert(!plugin->progress_visible);
        assert(plugin->progress_started_ns == progress_started_ns);
        assert(show_splash_screen(plugin, &loop, NULL, PLY_BOOT_SPLASH_MODE_BOOT_UP));
        assert(plugin->progress_visible);
        assert(plugin->progress_started_ns == progress_started_ns);
        assert(plugin->progress_fraction == progress_fraction);

        on_boot_progress(plugin, 16.0, 0.80);
        assert(plugin->progress_fraction == 0.80);
        on_boot_progress(plugin, 17.0, 0.60);
        assert(plugin->progress_fraction == 0.80);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(last_clip.x == 490L);
        assert(last_clip.width == 240U && last_clip.height == 10U);

        display_password(plugin, "Password", 0);
        assert(!plugin->progress_visible);
        assert(ttfx_state_reaction_active(&plugin->state));

        display_password(plugin, "Password", 4);
        display_normal(plugin);
        assert(plugin->progress_visible);
        become_idle(plugin, &trigger);
        assert(!plugin->progress_visible);
        assert(trigger.pulls == 0U);
        mock_loop_on_step = engine_steps + 2U;
        on_timeout(plugin, &loop);
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_FINAL);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        on_timeout(plugin, &loop);
        assert(trigger.pulls == 1U);
        destroy_plugin(plugin);
}

static void test_pending_validation_cancels_without_stale_prompt(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));

        display_password(plugin, "Password", 4);
        display_normal(plugin);
        unsigned int hides = entry_hides;
        unsigned int shows = entry_shows;
        display_password(plugin, "Other volume", 0);
        assert(!ttfx_state_password_pending(&plugin->state));
        assert(!ttfx_state_reaction_active(&plugin->state));
        assert(plugin->views->state.prompt_visible);
        assert(entry_hides == hides);
        assert(entry_shows == shows);

        display_normal(plugin); /* zero bullets is backspace/empty, not validation */
        assert(!ttfx_state_password_pending(&plugin->state));
        assert(plugin->state.prompt_mode == TTFX_PROMPT_NONE);
        assert(!plugin->views->state.prompt_visible);
        assert(entry_hides == hides + 1U);
        destroy_plugin(plugin);
}

static void test_pending_validation_expiry_hides_retained_entry(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(plugin != NULL);
        plugin->clock_ns = mock_fade_clock_ns;
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display_password(plugin, "Password", 4);
        display_normal(plugin);
        assert(plugin->views->state.prompt_visible);
        unsigned int hides = entry_hides;

        mock_fade_now_ns += PROGRESS_VALIDATION_TIMEOUT_NS;
        loop.timeout_handler(loop.timeout_data, &loop);
        assert(!ttfx_state_password_pending(&plugin->state));
        assert(plugin->state.prompt_mode == TTFX_PROMPT_NONE);
        assert(!plugin->views->state.prompt_visible);
        assert(entry_hides == hides + 1U);
        destroy_plugin(plugin);
}

static void test_static_mode_bounds_pending_and_error_reaction_timers(void)
{
        ply_key_file_t off = {
                .mode = "off", .enabled = "false", .effect = "off", .seed = "0"
        };
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(&off);
        assert(plugin != NULL);
        plugin->clock_ns = mock_fade_clock_ns;
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        assert(loop.timeout_watches == 0U);

        display_password(plugin, "Password", 4);
        display_normal(plugin);
        assert(plugin->timeout_scheduled);
        assert(loop.timeout_watches == 1U);
        mock_fade_now_ns += PROGRESS_VALIDATION_TIMEOUT_NS;
        loop.timeout_handler(loop.timeout_data, &loop);
        assert(!plugin->views->state.prompt_visible);
        assert(!plugin->timeout_scheduled);
        assert(loop.timeout_watches == 1U);

        display_password(plugin, "Password", 4);
        display_normal(plugin);
        display_password(plugin, "Password", 0);
        assert(ttfx_state_reaction_active(&plugin->state));
        assert(plugin->timeout_scheduled);
        assert(loop.timeout_watches == 2U);
        plugin->state.reaction_tick = TTFX_REACTION_TICKS - 1U;
        loop.timeout_handler(loop.timeout_data, &loop);
        assert(!ttfx_state_reaction_active(&plugin->state));
        assert(plugin->views->state.prompt_visible);
        assert(last_entry_show_x == (1280L - 286L) / 2L);
        assert(!plugin->timeout_scheduled);
        assert(loop.timeout_watches == 2U);
        destroy_plugin(plugin);
}

static void test_password_draw_uses_original_lock_and_bullets_without_prompt_words(void)
{
        ply_key_file_t config = {
                .image_dir = "/trusted/omarchy", .mode = "fixed", .enabled = "true",
                .effect = "decrypt", .seed = "7"
        };
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(&config);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        display_password(plugin, "Please enter passphrase", 4);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);

        long entry_x = (1280L - 286L) / 2L;
        long entry_y = (long)(plugin->views->geometry.grid_y +
                              plugin->views->geometry.grid_height + 24U);
        assert(image_loads == 1U);
        assert(image_resizes == 1U);
        assert(resized_image_width == 34L);
        assert(resized_image_height == 38L);
        assert(image_composites == 1U);
        assert(image_composite_x == entry_x - 34L - 15L);
        assert(image_composite_y == entry_y + (48L - 38L) / 2L);
        assert(last_bullet_count == 4);
        assert(entry_draws == 1U);
        assert(password_prompt_label_shows == 0U);
        assert(prompt_label_draws == 0U);

        destroy_plugin(plugin);
        assert(image_allocations == 2U);
        assert(image_frees == 2U);
}

static void test_wrong_password_composites_horizontal_red_reaction_without_engine_reset(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        mock_cells[0] = (TtfxCell){.codepoint = 0x2588U, .fg_rgba = 0xffffffffU,
                                   .flags = TTFX_CELL_FG};

        display_password(plugin, "Password", 4);
        display_normal(plugin);
        display_password(plugin, "Password", 0);
        assert(ttfx_state_reaction_active(&plugin->state));
        assert(engine_creates == 1U && engine_steps == 0U && engine_frees == 0U);

        long entry_x = (1280L - 286L) / 2L;
        long entry_y = (long)(plugin->views->geometry.grid_y +
                              plugin->views->geometry.grid_height + 24U);
        assert(last_entry_show_x == entry_x + ttfx_state_reaction_x_offset(&plugin->state));
        assert(last_entry_show_y == entry_y);
        for (unsigned int tick = 0U; tick < TTFX_REACTION_TICKS; tick++) {
                int expected_offset = ttfx_state_reaction_x_offset(&plugin->state);
                captured_count = 0U;
                glitch_red_fills = 0U;
                capture_geometry = true;
                display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
                capture_geometry = false;
                /* wrong answer: the padlock itself turns red */
                assert(last_fill_source == plugin->lock_buffer_error);
                /* the red padlock keeps the normal scaled size: it only
                 * tints, the shake comes from the entry's reaction offset */
                assert(plugin->lock_buffer_error->width == (unsigned long)resized_image_width);
                assert(plugin->lock_buffer_error->height == (unsigned long)resized_image_height);
                assert(captured_count == 8U);
                assert(captured_rects[1].x ==
                       (long)plugin->views->geometry.grid_x + expected_offset);
                assert(captured_rects[1].y == (long)plugin->views->geometry.grid_y);
                assert(glitch_red_fills == 7U);
                assert(captured_rects[2].x == entry_x + expected_offset);
                assert(captured_rects[2].y == entry_y);
                assert(captured_rects[2].width == 286U && captured_rects[2].height == 3U);
                assert(captured_rects[3].x == entry_x + expected_offset);
                assert(captured_rects[3].y == entry_y + 45L);
                assert(captured_rects[3].width == 286U && captured_rects[3].height == 3U);
                assert(captured_rects[4].x == entry_x + expected_offset);
                assert(captured_rects[4].y == entry_y);
                assert(captured_rects[4].width == 3U && captured_rects[4].height == 48U);
                assert(captured_rects[5].x == entry_x + expected_offset + 283L);
                assert(captured_rects[5].y == entry_y);
                assert(captured_rects[5].width == 3U && captured_rects[5].height == 48U);
                assert(captured_rects[6].y == entry_y + 11L);
                assert(captured_rects[7].y == entry_y + 33L);
                assert(image_composite_x == entry_x - 34L - 15L + expected_offset);
                assert(image_composite_y == entry_y + (48L - 38L) / 2L);
                assert(last_entry_show_x == entry_x + expected_offset);
                assert(last_entry_show_y == entry_y);
                /* submit accelerates towards the final frame; the retry
                 * keeps the same engine without a hard reset. */
                assert(engine_creates == 1U &&
                       engine_steps == tick * TTFX_FAST_FORWARD_STEPS_PER_TICK &&
                       engine_frees == 0U);
                loop.timeout_handler(loop.timeout_data, &loop);
        }
        assert(!ttfx_state_reaction_active(&plugin->state));
        assert(engine_creates == 1U &&
               engine_steps == TTFX_REACTION_TICKS * TTFX_FAST_FORWARD_STEPS_PER_TICK &&
               engine_frees == 0U);
        assert(plugin->phase.step == TTFX_REACTION_TICKS * TTFX_FAST_FORWARD_STEPS_PER_TICK);

        captured_count = 0U;
        glitch_red_fills = 0U;
        capture_geometry = true;
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        capture_geometry = false;
        assert(captured_rects[1].x == (long)plugin->views->geometry.grid_x);
        assert(captured_rects[1].y == (long)plugin->views->geometry.grid_y);
        assert(glitch_red_fills == 0U);
        assert(image_composite_x == entry_x - 34L - 15L);
        assert(image_composite_y == entry_y + (48L - 38L) / 2L);
        assert(last_entry_show_x == entry_x);
        assert(last_entry_show_y == entry_y);
        assert(password_prompt_label_shows == 0U);
        assert(prompt_label_draws == 0U);
        assert(message_label_draws == 0U);
        destroy_plugin(plugin);
}

static void test_error_overlay_recolors_only_existing_bullet_count(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        mock_cells[0] = (TtfxCell){.codepoint = 0x2588U, .fg_rgba = 0xffffffffU,
                                   .flags = TTFX_CELL_FG};
        display_password(plugin, "Password", 4);
        display_normal(plugin);
        display_password(plugin, "Password", 0);
        display_password(plugin, "Password", 3);

        captured_count = 0U;
        glitch_red_fills = 0U;
        capture_geometry = true;
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        capture_geometry = false;
        assert(glitch_red_fills == 10U); /* logo glyph + 6 box bands + 3 bullets */
        assert(captured_count == 11U);   /* background plus those ten red rectangles */
        for (size_t index = 8U; index < 11U; index++) {
                assert(captured_rects[index].width == 4U);
                assert(captured_rects[index].height == 4U);
        }
        assert(plugin->state.bullets == 3);
        destroy_plugin(plugin);
}

static void test_password_mode_retains_but_does_not_draw_ordinary_messages(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(plugin != NULL);
        plugin->clock_ns = mock_fade_clock_ns;
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display_password(plugin, "Password", 2);
        display_message(plugin, "Unlocking encrypted volume");
        assert(plugin->state.message_visible);
        assert(strcmp(plugin->state.message, "Unlocking encrypted volume") == 0);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(message_label_shows == 0U);
        assert(message_label_draws == 0U);

        display_normal(plugin);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(message_label_shows == 0U);
        assert(message_label_draws == 0U);
        mock_fade_now_ns += PROGRESS_VALIDATION_TIMEOUT_NS;
        loop.timeout_handler(loop.timeout_data, &loop);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(message_label_shows == 1U);
        assert(message_label_draws == 1U);
        destroy_plugin(plugin);
}

static void test_missing_lock_asset_uses_graphical_marker_without_losing_entry(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};

        reset_counters();
        mock_image_load_success = false;
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        display_password(plugin, "Password", 3);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(image_loads == 1U);
        assert(image_resizes == 0U);
        assert(image_composites == 0U);
        assert(logo_color_fills == 3U);
        assert(entry_draws == 1U);
        assert(last_bullet_count == 3);
        display_normal(plugin);
        assert(ttfx_state_password_pending(&plugin->state));
        assert(!plugin->progress_visible);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(entry_draws == 2U);
        assert(image_loads == 3U);
        destroy_plugin(plugin);
        assert(image_allocations == 3U);
        assert(image_frees == 3U);
}

static void test_question_prompt_keeps_required_text_label(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display_question(plugin, "Recovery key", "");
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(password_prompt_label_shows == 1U);
        assert(prompt_label_draws == 1U);
        destroy_plugin(plugin);
}

static void test_one_global_engine_advances_2x_per_240hz_timeout_not_draw(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t first = {.width = 1280U, .height = 720U};
        ply_pixel_display_t second = {.width = 800U, .height = 600U};
        ply_pixel_buffer_t buffer = {0};
        ply_boot_splash_plugin_t *plugin;

        reset_counters();
        plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &first);
        add_pixel_display(plugin, &second);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        assert(engine_creates == 1U);
        assert(strcmp(created_effect, "decrypt") == 0);
        assert(created_seed == UINT64_C(22321466108495961));
        assert(engine_cells_calls == 1U);
        assert(plugin->cells == mock_cells);
        assert(plugin->cell_count == 162U * 20U);
        assert(plugin->cells[0].codepoint == UINT32_C(0x4f));
        first.draw_handler(first.draw_data, &buffer, 0, 0, 1280, 720, &first);
        second.draw_handler(second.draw_data, &buffer, 0, 0, 800, 600, &second);
        assert(engine_steps == 0U);
        assert(engine_cells_calls == 1U);
        assert(loop.timeout_handler != NULL);
        unsigned int draws_before_tick = draw_requests;
        loop.timeout_handler(loop.timeout_data, &loop);
        assert(engine_steps == TTFX_PLAYBACK_STEPS_PER_TICK);
        assert(engine_cells_calls == 1U + TTFX_PLAYBACK_STEPS_PER_TICK);
        assert(loop.timeout_watches == 2U);
        assert(draw_requests == draws_before_tick + 2U);
        destroy_plugin(plugin);
        assert(engine_frees == 1U);
}

static void test_default_input_only_plays_once_and_freezes(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_trigger_t trigger = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display_password(plugin, "Password", 4);
        display_normal(plugin);
        assert(plugin->state.playback_mode == TTFX_PLAYBACK_SUBMIT_TO_FINISH);
        /* the submit accelerates towards the final frame for the verdict */
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_FAST_FORWARD);

        mock_loop_on_step = 1U;
        on_timeout(plugin, &loop);
        assert(engine_steps == 1U);
        assert(engine_resets == 0U);
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_FINAL);

        become_idle(plugin, &trigger);
        assert(trigger.pulls == 0U);
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_FINAL);
        on_timeout(plugin, &loop);
        assert(engine_steps == 1U);
        assert(engine_resets == 0U);
        destroy_plugin(plugin);
}

static void test_continuous_playback_keeps_the_previous_looping_behavior(void)
{
        ply_key_file_t configured = {
                .mode = "fixed", .enabled = "true", .effect = "decrypt", .seed = "7",
                .playback = "continuous"
        };
        ply_event_loop_t loop = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(&configured);
        assert(plugin->state.playback_mode == TTFX_PLAYBACK_CONTINUOUS);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display_password(plugin, "Password", 3);
        display_normal(plugin);
        mock_loop_on_step = 1U;
        on_timeout(plugin, &loop);
        assert(engine_steps == 1U);
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_INPUT);
        on_timeout(plugin, &loop);
        assert(engine_steps == 1U + TTFX_PLAYBACK_STEPS_PER_TICK);
        destroy_plugin(plugin);
}

static void test_wrong_password_does_not_restart_input_only_playback(void)
{
        ply_event_loop_t loop = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display_password(plugin, "Password", 5);
        display_normal(plugin);
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_FAST_FORWARD);
        mock_loop_on_step = 2U;
        on_timeout(plugin, &loop);
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_FINAL);

        display_password(plugin, "Password", 0);
        assert(ttfx_state_reaction_active(&plugin->state));
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_INPUT);
        assert(engine_resets == 0U);
        on_timeout(plugin, &loop);
        assert(engine_resets == 0U);
        destroy_plugin(plugin);
}

static void test_repeated_show_and_cross_loop_are_idempotent(void)
{
        ply_event_loop_t first_loop = {0};
        ply_event_loop_t second_loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_boot_splash_plugin_t *plugin;
        unsigned int loads_after_first_show;
        unsigned int entry_shows_after_first_show;
        unsigned int label_shows_after_first_show;
        unsigned int draws_after_first_show;

        reset_counters();
        plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        display_password(plugin, "Password", 3);
        assert(show_splash_screen(plugin, &first_loop, NULL, 0));
        assert(first_loop.exit_watches == 1U);
        assert(first_loop.timeout_watches == 1U);
        loads_after_first_show = entry_loads;
        entry_shows_after_first_show = entry_shows;
        label_shows_after_first_show = label_shows;
        draws_after_first_show = draw_requests;

        assert(show_splash_screen(plugin, &first_loop, NULL, 0));
        assert(first_loop.exit_watches == 1U);
        assert(first_loop.timeout_watches == 1U);
        assert(entry_loads == loads_after_first_show);
        assert(entry_shows == entry_shows_after_first_show);
        assert(label_shows == label_shows_after_first_show);
        assert(draw_requests == draws_after_first_show);

        assert(!show_splash_screen(plugin, &second_loop, NULL, 0));
        assert(plugin->loop == &first_loop);
        assert(plugin->visible);
        assert(first_loop.exit_watches == 1U);
        assert(first_loop.timeout_watches == 1U);
        assert(second_loop.exit_watches == 0U);
        assert(second_loop.timeout_watches == 0U);
        assert(entry_loads == loads_after_first_show);
        assert(entry_shows == entry_shows_after_first_show);
        assert(label_shows == label_shows_after_first_show);
        assert(draw_requests == draws_after_first_show);

        destroy_plugin(plugin);
        assert(first_loop.exit_stops == 1U);
        assert(first_loop.timeout_stops == 1U);
        assert(second_loop.exit_stops == 0U);
        assert(second_loop.timeout_stops == 0U);
        assert(entry_allocations == entry_frees);
        assert(label_allocations == label_frees);
}

static void test_duplicate_display_add_and_remove_are_idempotent(void)
{
        ply_pixel_display_t display = {.width = 1024U, .height = 768U};
        ply_boot_splash_plugin_t *plugin;

        reset_counters();
        plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(entry_allocations == 1U);
        assert(label_allocations == 2U);
        assert(draw_handler_sets == 1U);

        add_pixel_display(plugin, &display);
        assert(entry_allocations == 1U);
        assert(label_allocations == 2U);
        assert(draw_handler_sets == 1U);
        assert(plugin->views != NULL);
        assert(plugin->views->next == NULL);

        remove_pixel_display(plugin, &display);
        assert(plugin->views == NULL);
        assert(display.draw_handler == NULL);
        assert(draw_handler_clears == 1U);
        assert(entry_allocations == entry_frees);
        assert(label_allocations == label_frees);

        remove_pixel_display(plugin, &display);
        assert(draw_handler_clears == 1U);
        destroy_plugin(plugin);
        assert(draw_handler_clears == 1U);
}

static void test_step_error_degrades_to_static_without_losing_password_entry(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};
        ply_boot_splash_plugin_t *plugin;

        reset_counters();
        plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        display_password(plugin, "Password", 2);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        assert(label_color_sets == 2U);

        mock_step_status = TTFX_STATUS_ENGINE_ERROR;
        loop.timeout_handler(loop.timeout_data, &loop);
        assert(engine_steps == 1U);
        assert(engine_frees == 1U);
        assert(!plugin->state.animation_enabled);
        assert(plugin->cells == NULL);
        assert(loop.timeout_watches == 1U);

        display_password(plugin, "Retry", 5);
        assert(plugin->state.prompt_mode == TTFX_PROMPT_PASSWORD);
        assert(plugin->state.bullets == 5);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(entry_draws == 1U);
        destroy_plugin(plugin);
        assert(engine_frees == 1U);
}

static void test_single_step_tick_stops_on_failure_and_degraded_state(void)
{
        for (unsigned int fail = 1U; fail <= 1U; fail++) {
                ply_event_loop_t loop = {0};
                reset_counters();
                ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
                assert(plugin != NULL);
                assert(show_splash_screen(plugin, &loop, NULL, 0));
                mock_fail_on_step = fail;
                on_timeout(plugin, &loop);
                assert(engine_steps == fail);
                assert(engine_cells_calls == fail);
                assert(engine_frees == 1U);
                assert(plugin->static_degraded);
                assert(!plugin->timeout_scheduled);
                assert(loop.timeout_watches == 1U);
                on_timeout(plugin, &loop);
                assert(engine_steps == fail);
                destroy_plugin(plugin);
                assert(engine_frees == 1U);
        }
        ply_event_loop_t loop = {0};
        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        plugin->static_degraded = true;
        on_timeout(plugin, &loop);
        assert(engine_steps == 0U);
        assert(loop.timeout_watches == 1U);
        destroy_plugin(plugin);
}

static void test_step_error_hide_show_rebinds_static_prompt_without_retry(void)
{
        ply_event_loop_t first_loop = {0};
        ply_event_loop_t second_loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_boot_splash_plugin_t *plugin;
        unsigned int shows_before_hide;
        unsigned int label_shows_before_hide;
        unsigned int label_hides_before_hide;

        reset_counters();
        plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        display_password(plugin, "Password", 4);
        display_message(plugin, "Unlocking disk");
        assert(show_splash_screen(plugin, &first_loop, NULL, 0));
        mock_step_status = TTFX_STATUS_ENGINE_ERROR;
        first_loop.timeout_handler(first_loop.timeout_data, &first_loop);
        assert(engine_frees == 1U);
        shows_before_hide = entry_shows;
        label_shows_before_hide = label_shows;
        label_hides_before_hide = label_hides;

        hide_splash_screen(plugin, &first_loop);
        assert(entry_hides == 1U);
        assert(label_hides == label_hides_before_hide + 1U);
        assert(plugin->state.message_visible);
        assert(strcmp(plugin->state.message, "Unlocking disk") == 0);
        assert(first_loop.exit_stops == 1U);
        assert(show_splash_screen(plugin, &second_loop, NULL, 0));
        assert(plugin->visible);
        assert(plugin->loop == &second_loop);
        assert(entry_shows == shows_before_hide + 1U);
        assert(label_shows == label_shows_before_hide);
        assert(last_bullet_count == 4);
        assert(engine_creates == 1U);
        assert(second_loop.exit_watches == 1U);
        assert(second_loop.timeout_watches == 0U);

        destroy_plugin(plugin);
        assert(second_loop.exit_stops == 1U);
        assert(engine_frees == 1U);
}

static void test_hide_show_preserves_engine_and_resumes_timeout(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1024U, .height = 768U};
        ply_boot_splash_plugin_t *plugin;

        reset_counters();
        plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        hide_splash_screen(plugin, &loop);
        assert(engine_creates == 1U && engine_frees == 0U);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        assert(engine_creates == 1U);
        assert(loop.timeout_watches == 2U);
        destroy_plugin(plugin);
        assert(engine_frees == 1U);
}

static void test_renderer_uses_real_cell_colors_and_skips_space_and_hidden_foreground(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};
        ply_boot_splash_plugin_t *plugin;

        reset_counters();
        mock_cells[0] = (TtfxCell){.codepoint = 0x2588U, .fg_rgba = 0xff0000ffU,
                                   .flags = TTFX_CELL_FG};
        mock_cells[1] = (TtfxCell){.codepoint = ' ', .fg_rgba = 0x00ff00ffU,
                                   .flags = TTFX_CELL_FG};
        mock_cells[2] = (TtfxCell){.codepoint = 0x2588U, .fg_rgba = 0x00ff00ffU,
                                   .flags = TTFX_CELL_FG | TTFX_CELL_HIDDEN};
        mock_cells[3] = (TtfxCell){.codepoint = ' ', .bg_rgba = 0x0000ffffU,
                                   .flags = TTFX_CELL_BG};
        plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        mock_cells[0].codepoint = 0x2588U;
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(red_fills == 1U);
        assert(green_fills == 0U);
        assert(blue_fills == 1U);
        assert(engine_steps == 0U && engine_cells_calls == 1U);
        destroy_plugin(plugin);
}

static void test_renderer_swaps_foreground_and_background_for_reverse_cells(void)
{
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};
        ply_boot_splash_plugin_t *plugin;

        reset_counters();
        mock_cells[0] = (TtfxCell){.codepoint = 0x2588U,
                                   .fg_rgba = 0xff0000ffU,
                                   .bg_rgba = 0x0000ffffU,
                                   .flags = TTFX_CELL_FG | TTFX_CELL_BG | TTFX_CELL_REVERSE};
        plugin = create_plugin(NULL);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        mock_cells[0].codepoint = 0x2588U;
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(primary_fill_count == 2U);
        assert(primary_fill_order[0] == 0xff0000U);
        assert(primary_fill_order[1] == 0x0000ffU);
        destroy_plugin(plugin);
}

static void test_keyfile_contract_controls_engine_static_mode_and_colors(void)
{
        ply_key_file_t animated = {
                .image_dir = "/custom/images", .mode = "fixed", .enabled = "true", .effect = "waves",
                .seed = "18446744073709551615", .background = "ff0000",
                .text = "00ff00", .message = "0000ff"
        };
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};
        ply_boot_splash_plugin_t *plugin;

        reset_counters();
        plugin = create_plugin(&animated);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        assert(engine_creates == 1U);
        assert(strcmp(created_effect, "waves") == 0);
        assert(created_seed == UINT64_MAX);
        assert(created_background_rgb == UINT32_C(0xff0000));
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(red_fills > 0U);
        /* The static geometric mark must not overlay a live TTFX frame. */
        assert(green_fills == 0U);
        destroy_plugin(plugin);

        ply_key_file_t randomized = {
                .mode = "random", .enabled = "true", .effect = "matrix", .seed = "42"
        };
        ply_event_loop_t random_loop = {0};
        ply_pixel_display_t random_display = {.width = 1024U, .height = 768U};
        reset_counters();
        plugin = create_plugin(&randomized);
        assert(plugin != NULL);
        add_pixel_display(plugin, &random_display);
        assert(show_splash_screen(plugin, &random_loop, NULL, 0));
        assert(engine_creates == 1U);
        assert(strcmp(created_effect, "matrix") == 0);
        assert(created_seed == UINT64_C(42));
        destroy_plugin(plugin);

        ply_key_file_t off = {.mode = "off", .enabled = "false", .effect = "off", .seed = "0"};
        ply_event_loop_t first_loop = {0};
        ply_event_loop_t second_loop = {0};
        ply_pixel_display_t off_display = {.width = 1024U, .height = 768U};
        reset_counters();
        plugin = create_plugin(&off);
        assert(plugin != NULL);
        add_pixel_display(plugin, &off_display);
        display_password(plugin, "Password", 3);
        assert(show_splash_screen(plugin, &first_loop, NULL, 0));
        assert(engine_creates == 0U && first_loop.timeout_watches == 0U);
        hide_splash_screen(plugin, &first_loop);
        assert(show_splash_screen(plugin, &second_loop, NULL, 0));
        assert(engine_creates == 0U && second_loop.timeout_watches == 0U);
        assert(entry_shows == 2U);
        destroy_plugin(plugin);
}

static void test_malformed_keyfile_values_fall_back_to_safe_static_defaults(void)
{
        ply_key_file_t malformed = {
                .mode = "fixed", .enabled = "true ", .effect = "../waves", .seed = "18446744073709551616",
                .background = "ff0000x", .text = "00ff0g", .message = "12345"
        };
        ply_event_loop_t loop = {0};
        ply_pixel_display_t display = {.width = 800U, .height = 600U};
        ply_boot_splash_plugin_t *plugin;

        reset_counters();
        plugin = create_plugin(&malformed);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        assert(engine_creates == 0U && loop.timeout_watches == 0U);
        assert(plugin->background_color == BACKGROUND_COLOR);
        assert(plugin->text_color == LOGO_COLOR);
        assert(plugin->message_color == (MESSAGE_LABEL_COLOR >> 8U));
        destroy_plugin(plugin);
}

static void test_mode_contract_rejects_missing_invalid_and_contradictory_values(void)
{
        ply_key_file_t cases[] = {
                {.mode = NULL, .enabled = "true", .effect = "waves", .seed = "7"},
                {.mode = "current", .enabled = "true", .effect = "waves", .seed = "7"},
                {.mode = "fixed", .enabled = NULL, .effect = "waves", .seed = "7"},
                {.mode = "fixed", .enabled = "true", .effect = NULL, .seed = "7"},
                {.mode = "fixed", .enabled = "true", .effect = "waves", .seed = NULL},
                {.mode = "off", .enabled = "true", .effect = "waves", .seed = "7"},
                {.mode = "off", .enabled = "false", .effect = NULL, .seed = "0"},
                {.mode = "off", .enabled = "false", .effect = "off", .seed = "7"},
                {.mode = "fixed", .enabled = "false", .effect = "off", .seed = "0"},
                {.mode = "random", .enabled = "true", .effect = "off", .seed = "7"},
        };

        for (size_t index = 0U; index < sizeof(cases) / sizeof(cases[0]); index++) {
                ply_event_loop_t loop = {0};
                ply_pixel_display_t display = {.width = 1024U, .height = 768U};
                ply_boot_splash_plugin_t *plugin;

                reset_counters();
                plugin = create_plugin(&cases[index]);
                assert(plugin != NULL);
                add_pixel_display(plugin, &display);
                display_password(plugin, "Password", 2);
                assert(show_splash_screen(plugin, &loop, NULL, 0));
                assert(!plugin->state.animation_enabled);
                assert(engine_creates == 0U);
                assert(loop.timeout_watches == 0U);
                assert(entry_shows == 1U);
                destroy_plugin(plugin);
        }
}

static void test_unknown_effect_degrades_persistently_to_static_graphical_prompt(void)
{
        ply_key_file_t unknown = {
                .mode = "fixed", .enabled = "true", .effect = "unknown-effect", .seed = "9"
        };
        ply_event_loop_t first_loop = {0};
        ply_event_loop_t second_loop = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};
        ply_boot_splash_plugin_t *plugin;

        reset_counters();
        mock_create_status = TTFX_STATUS_INVALID_EFFECT;
        plugin = create_plugin(&unknown);
        assert(plugin != NULL);
        add_pixel_display(plugin, &display);
        display_password(plugin, "Password", 4);
        assert(show_splash_screen(plugin, &first_loop, NULL, 0));
        assert(plugin->visible);
        assert(plugin->static_degraded);
        assert(!plugin->state.animation_enabled);
        assert(engine_creates == 1U);
        assert(first_loop.timeout_watches == 0U);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(entry_draws == 1U);

        hide_splash_screen(plugin, &first_loop);
        assert(show_splash_screen(plugin, &second_loop, NULL, 0));
        assert(engine_creates == 1U);
        assert(second_loop.timeout_watches == 0U);
        assert(last_bullet_count == 4);
        destroy_plugin(plugin);
}

static void test_interface_accepts_required_status_updates(void)
{
        ply_boot_splash_plugin_interface_t *interface = ply_boot_splash_plugin_get_interface();

        assert(interface->update_status != NULL);
        assert(interface->on_boot_progress != NULL);
        interface->update_status(NULL, "Booting");
}

static void test_block_geometry_and_full_background(void)
{
        TtfxCell cells[8] = {0};
        ply_boot_splash_plugin_t plugin = {0};
        view_t view = {0};
        const uint32_t glyphs[] = {0x2588U, 0x2580U, 0x2584U, 0x2588U, ' ', 0U, 0x2584U, 0x2580U};
        for (size_t i = 0; i < 8U; i++) {
                cells[i].codepoint = glyphs[i];
                cells[i].flags = TTFX_CELL_FG;
                cells[i].fg_rgba = 0xffffffffU;
        }
        cells[6].flags |= TTFX_CELL_BG | TTFX_CELL_HIDDEN;
        cells[7].flags |= TTFX_CELL_BG | TTFX_CELL_REVERSE;
        plugin.cells = cells;
        plugin.cell_count = 8U;
        plugin.canvas_width = 8U;
        view.plugin = &plugin;
        view.geometry = ttfx_geometry_for_view(1920U, 1080U);
        view.geometry.columns = 8U;
        view.geometry.rows = 1U;
        capture_geometry = true;
        captured_count = 0U;
        draw_grid(&view, NULL);
        capture_geometry = false;
        assert(captured_count == 7U);
        long x = (long)view.geometry.grid_x;
        long y = (long)view.geometry.grid_y;
        assert(captured_rects[0].x == x && captured_rects[0].y == y);
        assert(captured_rects[0].width == 5U && captured_rects[0].height == 10U);
        assert(captured_rects[1].x == x + 5 && captured_rects[1].y == y);
        assert(captured_rects[1].width == 5U && captured_rects[1].height == 5U);
        assert(captured_rects[2].x == x + 10 && captured_rects[2].y == y + 5);
        assert(captured_rects[2].height == 5U);
        assert(captured_rects[3].x == x + 15 && captured_rects[3].height == 10U);
        /* Hidden glyph paints only its full background; reverse does likewise first. */
        assert(captured_rects[4].x == x + 30 && captured_rects[4].height == 10U);
        assert(captured_rects[5].x == x + 35 && captured_rects[5].height == 10U);
        assert(captured_rects[6].x == x + 35 && captured_rects[6].height == 5U);
}

static void test_letter_uses_detailed_glyph_mask(void)
{
        TtfxCell cells[1] = {{.codepoint = 'A', .fg_rgba = 0xffffffffU,
                              .flags = TTFX_CELL_FG}};
        ply_boot_splash_plugin_t plugin = {.cells = cells, .cell_count = 1U,
                                            .canvas_width = 1U};
        view_t view = {.plugin = &plugin};
        uint32_t pixels[50] = {0};
        ply_pixel_buffer_t buffer = {.width = 5U, .height = 10U, .scale = 1,
                                     .pixels = pixels};
        size_t foreground_pixels = 0U;

        view.geometry = (ttfx_geometry_t){.columns = 1U, .rows = 1U,
                                           .cell_width = 5U, .cell_height = 10U,
                                           .grid_width = 5U, .grid_height = 10U};
        draw_grid(&view, &buffer);
        for (size_t index = 0U; index < 50U; index++)
                if (pixels[index] == UINT32_C(0xffffffff))
                        foreground_pixels++;
        assert(foreground_pixels > 0U);
        assert(foreground_pixels < 50U);
}

void ply_trigger_pull(ply_trigger_t *trigger, void *data)
{
        assert(data == NULL);
        trigger->pulls++;
}

static uint64_t raw_little(const uint8_t *bytes, size_t count)
{
        uint64_t value = 0U;
        for (size_t index = 0U; index < count; index++)
                value |= (uint64_t)bytes[index] << (index * 8U);
        return value;
}

static void test_idle_publishes_clean_base_while_retained_password_is_visible(void)
{
        char directory[] = "/dev/shm/ttfx-idle-prompt.XXXXXX";
        assert(mkdtemp(directory) != NULL);
        ply_event_loop_t loop = {0};
        ply_trigger_t trigger = {0};
        ply_pixel_display_t display = {.width = 81U, .height = 10U};
        uint32_t pixels[810] = {0};
        ply_pixel_buffer_t buffer = {.width = 81U, .height = 10U, .scale = 1, .pixels = pixels};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        plugin->handoff_directory = open(directory, O_RDONLY | O_DIRECTORY);
        assert(plugin->handoff_directory >= 0);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display.draw_handler(display.draw_data, &buffer, 0, 0, 81, 10, &display);
        plugin->background_color = UINT32_C(0x2468ac);
        display_password(plugin, "Password", 4);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 81, 10, &display);
        assert(pixels[0] == SECRET_ENTRY_PIXEL);
        display_normal(plugin);
        assert(ttfx_state_password_pending(&plugin->state));
        assert(plugin->views->state.prompt_visible);
        unsigned int hides = entry_hides;
        plugin->clock_ns = mock_fade_clock_ns;

        become_idle(plugin, &trigger);
        assert(trigger.pulls == 0U);
        assert(plugin->state.prompt_mode == TTFX_PROMPT_NONE);
        assert(!ttfx_state_password_pending(&plugin->state));
        assert(!plugin->views->state.prompt_visible);
        assert(entry_hides == hides + 1U);
        mock_loop_on_step = engine_steps + 2U;
        on_timeout(plugin, &loop);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 81, 10, &display);
        assert(plugin->views->raw_valid);
        on_timeout(plugin, &loop);
        assert(trigger.pulls == 1U);
        int raw_file = openat(plugin->handoff_directory, TTFX_RAW_NAME, O_RDONLY | O_NOFOLLOW);
        uint8_t raw[48U + sizeof(pixels)];
        assert(raw_file >= 0);
        assert(read(raw_file, raw, sizeof(raw)) == (ssize_t)sizeof(raw));
        close(raw_file);
        assert(memcmp(raw, "OMBFRAW1", 8U) == 0);
        assert(raw_little(raw + 8U, 4U) == 81U && raw_little(raw + 12U, 4U) == 10U);
        assert(raw_little(raw + 48U, 4U) == UINT32_C(0x2468ac));
        for (size_t offset = 48U; offset < sizeof(raw); offset += 4U)
                assert(raw_little(raw + offset, 4U) != (SECRET_ENTRY_PIXEL & UINT32_C(0xffffff)));
        ttfx_handoff_clear(plugin->handoff_directory);
        destroy_plugin(plugin);
        assert(rmdir(directory) == 0);
}

static void test_idle_freezes_and_completes_trigger(void)
{
        ply_key_file_t configured = {
                .mode = "fixed", .enabled = "true", .effect = "decrypt", .seed = "7",
                .playback = "continuous"
        };
        ply_event_loop_t loop = {0};
        ply_trigger_t trigger = {0};
        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(&configured);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        ply_boot_splash_plugin_interface_t *iface = ply_boot_splash_plugin_get_interface();
        assert(iface->become_idle != NULL);
        iface->become_idle(plugin, &trigger);
        assert(trigger.pulls == 1U);
        assert(!plugin->timeout_scheduled);
        on_timeout(plugin, &loop);
        assert(engine_steps == 0U);
        destroy_plugin(plugin);
}

static void test_default_success_waits_for_terminal_frame_to_be_drawn(void)
{
        ply_event_loop_t loop = {0};
        ply_trigger_t trigger = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display_password(plugin, "Password", 4);
        display_normal(plugin);
        become_idle(plugin, &trigger);
        assert(trigger.pulls == 0U);
        assert(plugin->success_pending);
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_FAST_FORWARD);

        mock_loop_on_step = 3U;
        on_timeout(plugin, &loop);
        assert(plugin->state.playback_phase == TTFX_PLAYBACK_FINAL);
        assert(trigger.pulls == 0U);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(plugin->drawn_phase.step == plugin->phase.step);
        on_timeout(plugin, &loop);
        assert(trigger.pulls == 1U);
        assert(plugin->idle);
        assert(!plugin->success_pending);
        destroy_plugin(plugin);
}

static void test_idle_captures_drawn_not_pending_phase(void)
{
        ply_key_file_t configured = {
                .mode = "fixed", .enabled = "true", .effect = "decrypt", .seed = "7",
                .playback = "continuous"
        };
        ply_event_loop_t loop = {0};
        ply_trigger_t trigger = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};
        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(&configured);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        mock_loop_on_step = 4U;
        on_timeout(plugin, &loop);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        assert(plugin->drawn_phase.step == 2U && plugin->drawn_phase.cycle == 0U);
        TtfxCell visible = plugin->cells[0];
        on_timeout(plugin, &loop); /* damage is queued, not drawn */
        mock_cells[0].codepoint = 'X';
        become_idle(plugin, &trigger);
        assert(plugin->phase.step == 0U && plugin->phase.cycle == 1U);
        assert(plugin->drawn_phase.step == 2U);
        assert(!plugin->handoff_published); /* unprivileged /run: visual freeze still succeeds */
        assert(plugin->cells[0].codepoint == visible.codepoint);
        on_timeout(plugin, &loop);
        assert(engine_steps == 4U);
        become_idle(plugin, &trigger);
        assert(trigger.pulls == 2U);
        destroy_plugin(plugin);
}

static void test_idle_publishes_exact_phase_once(void)
{
        ply_key_file_t configured = {
                .mode = "fixed", .enabled = "true", .effect = "decrypt", .seed = "7",
                .playback = "continuous"
        };
        char directory[] = "/dev/shm/ttfx-idle-test.XXXXXX";
        assert(mkdtemp(directory) != NULL);
        ply_event_loop_t loop = {0};
        ply_trigger_t trigger = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        ply_pixel_buffer_t buffer = {0};
        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(&configured);
        plugin->handoff_directory = open(directory, O_RDONLY | O_DIRECTORY);
        assert(plugin->handoff_directory >= 0);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        on_timeout(plugin, &loop);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720, &display);
        on_timeout(plugin, &loop);
        become_idle(plugin, &trigger);
        assert(plugin->handoff_published);
        int file = openat(plugin->handoff_directory, TTFX_HANDOFF_NAME, O_RDONLY);
        char text[TTFX_HANDOFF_MAX_BYTES] = {0};
        assert(file >= 0 && read(file, text, sizeof(text) - 1) > 0);
        close(file);
        assert(strstr(text, "cycle=0\nstep=2\n") != NULL);
        assert(strstr(text, "fps=240\nspeed=2\n") != NULL);
        assert(strstr(text, "playback=continuous\n") != NULL);
        ttfx_handoff_clear(plugin->handoff_directory);
        become_idle(plugin, &trigger);
        assert(faccessat(plugin->handoff_directory, TTFX_HANDOFF_NAME, F_OK, 0) != 0);
        assert(trigger.pulls == 2U);
        destroy_plugin(plugin);
        assert(rmdir(directory) == 0);
}

static void test_raw_capture_guards_and_freeze(void)
{
        ply_key_file_t configured = {
                .mode = "fixed", .enabled = "true", .effect = "decrypt", .seed = "7",
                .playback = "continuous"
        };
        for (int mode = 0; mode < 9; mode++) {
                char directory[] = "/dev/shm/ttfx-raw-idle.XXXXXX";
                assert(mkdtemp(directory));
                reset_counters();
                ply_event_loop_t loop = {0};
                ply_trigger_t trigger = {0};
                ply_pixel_display_t display = {.width = 81, .height = 10};
                ply_pixel_display_t second = {.width = 81, .height = 10};
                uint32_t pixels[810];
                for (size_t i = 0; i < 810; i++) pixels[i] = 0xff123456U + (uint32_t)i;
                ply_pixel_buffer_t buffer = {.width = 81, .height = 10, .scale = 1, .pixels = pixels};
                ply_boot_splash_plugin_t *plugin = create_plugin(&configured);
                plugin->handoff_directory = open(directory, O_RDONLY | O_DIRECTORY);
                assert(plugin->handoff_directory >= 0);
                add_pixel_display(plugin, &display);
                assert(show_splash_screen(plugin, &loop, NULL, 0));
                if (mode == 1) display_password(plugin, "secret prompt", 4);
                if (mode == 2) display_message(plugin, "private message");
                if (mode == 3) add_pixel_display(plugin, &second);
                if (mode == 4) buffer.scale = 0;
                if (mode == 5) buffer.rotation = 1;
                display.draw_handler(display.draw_data, &buffer, 0, 0, 81, 10, &display);
                if (mode == 2) assert(pixels[1] == SECRET_LABEL_PIXEL);
                if (mode == 6) display.draw_handler(display.draw_data, &buffer, 0, 0, 1, 1, &display);
                if (mode == 7) display_password(plugin, "pending password", 8);
                if (mode == 8) {
                        display_password(plugin, "old password", 2);
                        display.draw_handler(display.draw_data, &buffer, 0, 0, 81, 10, &display);
                        display_normal(plugin); /* queued erase is NOT proof of removal */
                }
                on_timeout(plugin, &loop); /* pending newer phase must not be captured */
                pixels[0] = 0; /* capture owns its data, not a borrowed pointer */
                plugin->clock_ns = mock_fade_clock_ns;
                become_idle(plugin, &trigger);
                int fd = openat(plugin->handoff_directory, TTFX_RAW_NAME, O_RDONLY);
                if (mode <= 2 || mode == 6 || mode >= 7) {
                        assert(fd >= 0);
                        uint8_t raw[48 + sizeof(pixels)];
                        assert(read(fd, raw, sizeof(raw)) == sizeof(raw));
                        assert(memcmp(raw, "OMBFRAW1", 8) == 0);
                        assert(raw[48] == 0x10 && raw[49] == 0x0d && raw[50] == 0x0b && raw[51] == 0);
                        for (size_t offset = 48U; offset < sizeof(raw); offset += 4U)
                                assert(raw_little(raw + offset, 4U) !=
                                               (SECRET_ENTRY_PIXEL & UINT32_C(0xffffff)) &&
                                       raw_little(raw + offset, 4U) !=
                                               (SECRET_LABEL_PIXEL & UINT32_C(0xffffff)));
                        assert(plugin->drawn_phase.step == 0);
                        assert(plugin->phase.step == TTFX_PLAYBACK_STEPS_PER_TICK);
                        close(fd);
                } else assert(fd < 0);
                ttfx_handoff_clear(plugin->handoff_directory);
                destroy_plugin(plugin);
                assert(rmdir(directory) == 0);
        }
}

static void test_reaction_never_replaces_clean_raw_with_unencoded_glitch_pixels(void)
{
        ply_key_file_t configured = {
                .mode = "fixed", .enabled = "true", .effect = "decrypt", .seed = "7",
                .playback = "continuous"
        };
        for (int advance_phase = 0; advance_phase <= 1; advance_phase++) {
                char directory[] = "/dev/shm/ttfx-reaction-raw.XXXXXX";
                assert(mkdtemp(directory) != NULL);
                reset_counters();
                ply_event_loop_t loop = {0};
                ply_trigger_t trigger = {0};
                ply_pixel_display_t display = {.width = 81U, .height = 10U};
                uint32_t pixels[810] = {0};
                ply_pixel_buffer_t buffer = {
                        .width = 81U, .height = 10U, .scale = 1, .pixels = pixels
                };
                ply_boot_splash_plugin_t *plugin = create_plugin(&configured);
                plugin->handoff_directory = open(directory, O_RDONLY | O_DIRECTORY);
                assert(plugin->handoff_directory >= 0);
                add_pixel_display(plugin, &display);
                assert(show_splash_screen(plugin, &loop, NULL, 0));
                display.draw_handler(display.draw_data, &buffer, 0, 0, 81, 10, &display);

                display_password(plugin, "Password", 4);
                display_normal(plugin);
                display_password(plugin, "Password", 0);
                assert(ttfx_state_reaction_active(&plugin->state));
                if (advance_phase)
                        on_timeout(plugin, &loop);
                plugin->background_color = UINT32_C(0x765432);
                display.draw_handler(display.draw_data, &buffer, 0, 0, 81, 10, &display);

                become_idle(plugin, &trigger);
                int raw_file = openat(plugin->handoff_directory, TTFX_RAW_NAME,
                                      O_RDONLY | O_NOFOLLOW);
                if (!advance_phase) {
                        uint8_t first_pixel[52U];
                        assert(raw_file >= 0);
                        assert(read(raw_file, first_pixel, sizeof(first_pixel)) ==
                               (ssize_t)sizeof(first_pixel));
                        assert(raw_little(first_pixel + 48U, 4U) == BACKGROUND_COLOR);
                        close(raw_file);
                } else {
                        assert(raw_file < 0);
                }
                ttfx_handoff_clear(plugin->handoff_directory);
                destroy_plugin(plugin);
                assert(rmdir(directory) == 0);
        }
}

static void test_hidpi_idle_publishes_complete_physical_raw(void)
{
        ply_key_file_t configured = {
                .mode = "fixed", .enabled = "true", .effect = "decrypt", .seed = "7",
                .playback = "continuous"
        };
        char directory[] = "/dev/shm/ttfx-hidpi-raw.XXXXXX";
        assert(mkdtemp(directory) != NULL);
        ply_event_loop_t loop = {0};
        ply_trigger_t trigger = {0};
        ply_pixel_display_t display = {.width = 81U, .height = 10U};
        const size_t physical_width = 162U;
        const size_t physical_height = 20U;
        const size_t physical_pixels = physical_width * physical_height;
        uint32_t *pixels = calloc(physical_pixels, sizeof(*pixels));
        assert(pixels != NULL);
        pixels[physical_pixels - 1U] = UINT32_C(0x00abcdef);
        ply_pixel_buffer_t buffer = {
                .width = 81U, .height = 10U, .scale = 2, .pixels = pixels
        };

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(&configured);
        plugin->handoff_directory = open(directory, O_RDONLY | O_DIRECTORY);
        assert(plugin->handoff_directory >= 0);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display.draw_handler(display.draw_data, &buffer, 0, 0, 81, 10, &display);
        become_idle(plugin, &trigger);

        int raw_file = openat(plugin->handoff_directory, TTFX_RAW_NAME,
                              O_RDONLY | O_NOFOLLOW);
        uint8_t header[48U];
        uint8_t last_pixel[4U];
        struct stat info;
        assert(raw_file >= 0);
        assert(fstat(raw_file, &info) == 0);
        assert(info.st_size == (off_t)(sizeof(header) + physical_pixels * 4U));
        assert(read(raw_file, header, sizeof(header)) == (ssize_t)sizeof(header));
        assert(raw_little(header + 8U, 4U) == physical_width);
        assert(raw_little(header + 12U, 4U) == physical_height);
        assert(raw_little(header + 16U, 4U) == physical_width * 4U);
        assert(pread(raw_file, last_pixel, sizeof(last_pixel),
                     (off_t)(sizeof(header) + (physical_pixels - 1U) * 4U)) ==
               (ssize_t)sizeof(last_pixel));
        assert(raw_little(last_pixel, sizeof(last_pixel)) == UINT32_C(0x00abcdef));
        close(raw_file);
        ttfx_handoff_clear(plugin->handoff_directory);
        destroy_plugin(plugin);
        free(pixels);
        assert(rmdir(directory) == 0);
}

static void test_successful_password_finishes_without_green_fade(void)
{
        char directory[] = "/dev/shm/ttfx-success-no-green.XXXXXX";
        assert(mkdtemp(directory) != NULL);
        ply_event_loop_t loop = {0};
        ply_trigger_t trigger = {0};
        ply_pixel_display_t display = {.width = 1280U, .height = 720U};
        uint32_t *pixels = calloc(1280U * 720U, sizeof(*pixels));
        assert(pixels != NULL);
        ply_pixel_buffer_t buffer = {
                .width = 1280U, .height = 720U, .scale = 1, .pixels = pixels
        };

        reset_counters();
        ply_boot_splash_plugin_t *plugin = create_plugin(NULL);
        plugin->handoff_directory = open(directory, O_RDONLY | O_DIRECTORY);
        assert(plugin->handoff_directory >= 0);
        add_pixel_display(plugin, &display);
        assert(show_splash_screen(plugin, &loop, NULL, 0));
        display_password(plugin, "Password", 4);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720,
                             &display);
        assert(plugin->views->raw_valid);
        display_normal(plugin);
        assert(plugin->progress_visible);
        const unsigned int green_before = success_green_fills;
        const unsigned int opacity_before = opacity_composites;

        become_idle(plugin, &trigger);
        assert(trigger.pulls == 0U);
        assert(!plugin->progress_visible);
        assert(plugin->state.bullets == 0);
        mock_loop_on_step = engine_steps + 2U;
        on_timeout(plugin, &loop);
        display.draw_handler(display.draw_data, &buffer, 0, 0, 1280, 720,
                             &display);
        assert(success_green_fills == green_before);
        assert(opacity_composites == opacity_before);
        on_timeout(plugin, &loop);
        assert(trigger.pulls == 1U);
        int raw_file = openat(plugin->handoff_directory, TTFX_RAW_NAME,
                              O_RDONLY | O_NOFOLLOW);
        assert(raw_file >= 0);
        close(raw_file);

        ttfx_handoff_clear(plugin->handoff_directory);
        destroy_plugin(plugin);
        free(pixels);
        assert(rmdir(directory) == 0);
}

int main(void)
{
        test_pending_validation_keeps_password_entry_visible_until_result();
        test_original_progress_bar_replaces_pending_input_and_is_monotonic();
        test_pending_validation_cancels_without_stale_prompt();
        test_pending_validation_expiry_hides_retained_entry();
        test_static_mode_bounds_pending_and_error_reaction_timers();
        test_password_draw_uses_original_lock_and_bullets_without_prompt_words();
        test_wrong_password_composites_horizontal_red_reaction_without_engine_reset();
        test_error_overlay_recolors_only_existing_bullet_count();
        test_password_mode_retains_but_does_not_draw_ordinary_messages();
        test_missing_lock_asset_uses_graphical_marker_without_losing_entry();
        test_question_prompt_keeps_required_text_label();
        test_raw_capture_guards_and_freeze();
        test_hidpi_idle_publishes_complete_physical_raw();
        test_reaction_never_replaces_clean_raw_with_unencoded_glitch_pixels();
        test_idle_publishes_clean_base_while_retained_password_is_visible();
        test_idle_publishes_exact_phase_once();
        test_idle_captures_drawn_not_pending_phase();
        test_successful_password_finishes_without_green_fade();
        test_idle_freezes_and_completes_trigger();
        test_default_success_waits_for_terminal_frame_to_be_drawn();
        test_single_step_tick_stops_on_failure_and_degraded_state();
        test_block_geometry_and_full_background();
        test_letter_uses_detailed_glyph_mask();
        test_one_global_engine_advances_2x_per_240hz_timeout_not_draw();
        test_default_input_only_plays_once_and_freezes();
        test_continuous_playback_keeps_the_previous_looping_behavior();
        test_wrong_password_does_not_restart_input_only_playback();
        test_repeated_show_and_cross_loop_are_idempotent();
        test_duplicate_display_add_and_remove_are_idempotent();
        test_step_error_degrades_to_static_without_losing_password_entry();
        test_step_error_hide_show_rebinds_static_prompt_without_retry();
        test_hide_show_preserves_engine_and_resumes_timeout();
        test_renderer_uses_real_cell_colors_and_skips_space_and_hidden_foreground();
        test_renderer_swaps_foreground_and_background_for_reverse_cells();
        test_keyfile_contract_controls_engine_static_mode_and_colors();
        test_malformed_keyfile_values_fall_back_to_safe_static_defaults();
        test_mode_contract_rejects_missing_invalid_and_contradictory_values();
        test_unknown_effect_degrades_persistently_to_static_graphical_prompt();
        test_interface_accepts_required_status_updates();
        puts("plugin lifecycle tests: PASS");
        return 0;
}
