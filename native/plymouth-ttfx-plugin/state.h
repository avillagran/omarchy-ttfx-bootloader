#ifndef TTFX_PLYMOUTH_STATE_H
#define TTFX_PLYMOUTH_STATE_H

#include <stdbool.h>
#include <stdint.h>

/* Step zero is the snapshot returned by create/reset (already first next_frame).
 * Loops reconstruct that same tuple, so cycle is informational, not replay work. */
typedef struct {
        uint64_t step;
        uint64_t cycle;
} ttfx_phase_t;

#define TTFX_HANDOFF_NAME "omarchy-plymouth-handoff.state"
#define TTFX_HANDOFF_TEMP ".omarchy-plymouth-handoff.state.tmp"
#define TTFX_RAW_NAME "omarchy-plymouth-handoff.raw"
#define TTFX_RAW_TEMP ".omarchy-plymouth-handoff.raw.tmp"
#define TTFX_RAW_MAX_WIDTH 4096U
#define TTFX_RAW_MAX_HEIGHT 2160U
uint64_t ttfx_boottime_ns(void);
void ttfx_raw_clear(int directory);
bool ttfx_raw_publish(int directory, const uint32_t *pixels, uint32_t width,
                      uint32_t height, uint64_t captured_ns);
#define TTFX_HANDOFF_MAX_BYTES 512U
#define TTFX_HANDOFF_MAX_STEP UINT64_C(10000)
typedef struct {
        const char *effect;
        uint64_t seed;
        ttfx_phase_t phase;
        uint32_t width, height, fps, speed, background, foreground;
} ttfx_handoff_t;
/* open verifies fixed /run, root ownership and tmpfs. No path/env overrides.
 * publish accepts only that trusted directory fd in production. */
int ttfx_handoff_open(void);
bool ttfx_handoff_publish(int directory, const ttfx_handoff_t *handoff);
void ttfx_handoff_clear(int directory);
void ttfx_handoff_close(int directory);

#define TTFX_FRAME_COUNT 120U
#define TTFX_GRID_COLUMNS 162U
#define TTFX_GRID_ROWS 20U
#define TTFX_REACTION_TICKS 168U
#define TTFX_FAILURE_CANDIDATE_TICKS 1200U
#define TTFX_REACTION_OFFSET_TICKS 24U

typedef enum {
        TTFX_PROMPT_NONE,
        TTFX_PROMPT_PASSWORD,
        TTFX_PROMPT_QUESTION
} ttfx_prompt_mode_t;

typedef struct ttfx_message ttfx_message_t;

typedef struct {
        unsigned int frame;
        ttfx_prompt_mode_t prompt_mode;
        int bullets;
        char *prompt;
        char *entry_text;
        char *failure_candidate_prompt;
        char *message;
        ttfx_message_t *messages;
        bool message_visible;
        bool animation_enabled;
        bool rendering_failed;
        bool failure_candidate_ready;
        bool reaction_active;
        unsigned int failure_candidate_tick;
        unsigned int reaction_tick;
} ttfx_state_t;

typedef struct {
        bool prompt_visible;
        bool message_visible;
        bool prompt_label_available;
        bool message_label_available;
} ttfx_view_state_t;

typedef struct {
        bool exit_watch_attached;
} ttfx_loop_state_t;

typedef struct {
        bool required_widgets_ready;
} ttfx_load_state_t;

typedef struct {
        unsigned int columns;
        unsigned int rows;
        unsigned int cell_width;
        unsigned int cell_height;
        unsigned int grid_x;
        unsigned int grid_y;
        unsigned int grid_width;
        unsigned int grid_height;
} ttfx_geometry_t;

ttfx_state_t ttfx_state_initial(void);
void ttfx_state_destroy(ttfx_state_t *state);
void ttfx_state_tick(ttfx_state_t *state);
void ttfx_state_disable_animation(ttfx_state_t *state);
void ttfx_state_fail_rendering(ttfx_state_t *state);
void ttfx_state_set_normal(ttfx_state_t *state);
void ttfx_state_clear_prompt(ttfx_state_t *state);
bool ttfx_state_set_password(ttfx_state_t *state, const char *prompt, int bullets);
bool ttfx_state_password_pending(const ttfx_state_t *state);
bool ttfx_state_reaction_active(const ttfx_state_t *state);
int ttfx_state_reaction_x_offset(const ttfx_state_t *state);
bool ttfx_state_reaction_row_red(const ttfx_state_t *state, unsigned int row);
bool ttfx_state_set_question(ttfx_state_t *state, const char *prompt, const char *entry_text);
bool ttfx_state_set_message(ttfx_state_t *state, const char *message);
bool ttfx_state_hide_message(ttfx_state_t *state, const char *message);
ttfx_view_state_t ttfx_view_state_initial(void);
bool ttfx_view_show_prompt(ttfx_view_state_t *state);
bool ttfx_view_hide_prompt(ttfx_view_state_t *state);
bool ttfx_view_show_message(ttfx_view_state_t *state);
bool ttfx_view_hide_message(ttfx_view_state_t *state);
bool ttfx_view_prompt_can_show(const ttfx_view_state_t *state);
bool ttfx_view_message_can_show(const ttfx_view_state_t *state);
void ttfx_view_degrade_prompt(ttfx_state_t *plugin_state, ttfx_view_state_t *view_state);
void ttfx_view_degrade_message(ttfx_state_t *plugin_state, ttfx_view_state_t *view_state);
ttfx_loop_state_t ttfx_loop_state_initial(void);
void ttfx_loop_watch_attached(ttfx_loop_state_t *state);
bool ttfx_loop_detach_watch(ttfx_loop_state_t *state);
ttfx_load_state_t ttfx_load_state_initial(void);
void ttfx_load_state_record_view(ttfx_load_state_t *state, bool loaded);
bool ttfx_load_state_can_show(const ttfx_load_state_t *state);
bool ttfx_dynamic_view_should_keep(bool loaded);
ttfx_geometry_t ttfx_geometry_for_view(unsigned int width, unsigned int height);
unsigned int ttfx_geometry_x_edge(const ttfx_geometry_t *geometry, unsigned int column);


#endif
