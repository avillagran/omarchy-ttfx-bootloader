#include <assert.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

#include "state.h"

static void test_tick_progresses_and_wraps(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(state.frame == 0U);
        for (unsigned int i = 0; i < TTFX_FRAME_COUNT - 1U; i++)
                ttfx_state_tick(&state);
        assert(state.frame == TTFX_FRAME_COUNT - 1U);
        ttfx_state_tick(&state);
        assert(state.frame == 0U);
}

static void test_input_only_freezes_after_first_natural_completion(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(state.playback_mode == TTFX_PLAYBACK_SUBMIT_TO_FINISH);
        assert(state.playback_phase == TTFX_PLAYBACK_INPUT);
        assert(ttfx_state_engine_steps_per_tick(&state) == 2U);

        assert(ttfx_state_set_password(&state, "Password", 4));
        ttfx_state_set_normal(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_FAST_FORWARD);
        assert(ttfx_state_engine_steps_per_tick(&state) ==
               TTFX_FAST_FORWARD_STEPS_PER_TICK);

        ttfx_state_engine_completed(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_FINAL);
        assert(ttfx_state_engine_steps_per_tick(&state) == 0U);

        ttfx_state_confirm_success(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_FINAL);
        assert(ttfx_state_engine_steps_per_tick(&state) == 0U);
        ttfx_state_destroy(&state);
}

static void test_input_only_fast_forwards_when_success_precedes_completion(void)
{
        ttfx_state_t state = ttfx_state_initial();

        ttfx_state_confirm_success(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_FAST_FORWARD);
        assert(ttfx_state_engine_steps_per_tick(&state) ==
               TTFX_FAST_FORWARD_STEPS_PER_TICK);
        ttfx_state_engine_completed(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_FINAL);
        assert(ttfx_state_engine_steps_per_tick(&state) == 0U);
        ttfx_state_destroy(&state);
}

static void test_continuous_mode_preserves_current_looping_behavior(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_playback_mode(&state, "continuous"));
        assert(state.playback_mode == TTFX_PLAYBACK_CONTINUOUS);
        assert(ttfx_state_set_password(&state, "Password", 3));
        ttfx_state_set_normal(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_INPUT);
        assert(ttfx_state_engine_steps_per_tick(&state) == 2U);
        ttfx_state_engine_completed(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_INPUT);
        assert(ttfx_state_engine_steps_per_tick(&state) == 2U);
        assert(!ttfx_state_set_playback_mode(&state, "unknown"));
        ttfx_state_destroy(&state);
}

static void test_retry_returns_submit_to_finish_mode_to_input(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_password(&state, "Password", 5));
        ttfx_state_set_normal(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_FAST_FORWARD);
        ttfx_state_engine_completed(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_FINAL);
        assert(ttfx_state_set_password(&state, "Password", 0));
        assert(ttfx_state_reaction_active(&state));
        assert(state.playback_phase == TTFX_PLAYBACK_INPUT);
        assert(ttfx_state_engine_steps_per_tick(&state) == 2U);
        ttfx_state_destroy(&state);
}

static void test_submit_accelerates_to_final_for_verdict(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_password(&state, "Password", 4));
        ttfx_state_set_normal(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_FAST_FORWARD);
        assert(ttfx_state_engine_steps_per_tick(&state) ==
               TTFX_FAST_FORWARD_STEPS_PER_TICK);
        ttfx_state_destroy(&state);
}

static void test_failed_answer_mid_fast_forward_loops_after_final(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_password(&state, "Password", 4));
        ttfx_state_set_normal(&state);
        assert(ttfx_state_set_password(&state, "Password", 0));
        assert(ttfx_state_reaction_active(&state));
        assert(state.playback_phase == TTFX_PLAYBACK_FAST_FORWARD);
        ttfx_state_engine_completed(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_FINAL);
        ttfx_state_tick(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_INPUT);
        assert(ttfx_state_engine_steps_per_tick(&state) == 2U);
        ttfx_state_destroy(&state);
}

static void test_empty_submit_does_not_accelerate(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_password(&state, "Password", 0));
        ttfx_state_set_normal(&state);
        assert(state.playback_phase == TTFX_PLAYBACK_INPUT);
        ttfx_state_destroy(&state);
}

static void test_failed_nonempty_password_replacement_triggers_once(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_password(&state, "Password", 4));
        ttfx_state_set_normal(&state);
        assert(ttfx_state_password_pending(&state));
        assert(state.prompt_mode == TTFX_PROMPT_PASSWORD);
        assert(state.bullets == 4);
        assert(strcmp(state.prompt, "Password") == 0);
        assert(ttfx_state_set_password(&state, "Password", 0));
        assert(!ttfx_state_password_pending(&state));
        assert(ttfx_state_reaction_active(&state));
        assert(state.reaction_tick == 0U);

        ttfx_state_tick(&state);
        assert(state.reaction_tick == 1U);
        assert(ttfx_state_set_password(&state, "Password", 0));
        assert(state.reaction_tick == 1U);
        ttfx_state_destroy(&state);
}

static void test_password_candidate_expires_to_normal(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_password(&state, "Password", 4));
        ttfx_state_set_normal(&state);
        for (unsigned int tick = 1U; tick < TTFX_FAILURE_CANDIDATE_TICKS; tick++) {
                ttfx_state_tick(&state);
                assert(ttfx_state_password_pending(&state));
                assert(state.prompt_mode == TTFX_PROMPT_PASSWORD);
                assert(state.bullets == 4);
        }
        ttfx_state_tick(&state);
        assert(!ttfx_state_password_pending(&state));
        assert(state.prompt_mode == TTFX_PROMPT_NONE);
        assert(state.bullets == 0);
        assert(state.prompt == NULL);
        ttfx_state_destroy(&state);
}

static void test_clear_prompt_never_arms_password_failure(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_password(&state, "Password", 4));
        ttfx_state_clear_prompt(&state);
        assert(!ttfx_state_password_pending(&state));
        assert(state.prompt_mode == TTFX_PROMPT_NONE);
        assert(state.bullets == 0);
        assert(state.prompt == NULL);
        ttfx_state_destroy(&state);
}

static void test_password_failure_sequence_rejects_false_positives(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_password(&state, "Password", 4));
        assert(ttfx_state_set_password(&state, "Password", 0));
        assert(!ttfx_state_reaction_active(&state));

        assert(ttfx_state_set_password(&state, "Password", 3));
        ttfx_state_set_normal(&state);
        assert(ttfx_state_set_password(&state, "Other volume", 0));
        assert(!ttfx_state_reaction_active(&state));

        assert(ttfx_state_set_password(&state, "Password", 2));
        ttfx_state_set_normal(&state);
        for (unsigned int tick = 0U; tick < TTFX_FAILURE_CANDIDATE_TICKS; tick++)
                ttfx_state_tick(&state);
        assert(ttfx_state_set_password(&state, "Password", 0));
        assert(!ttfx_state_reaction_active(&state));

        assert(ttfx_state_set_password(&state, "Password", 0));
        ttfx_state_set_normal(&state);
        assert(ttfx_state_set_password(&state, "Password", 0));
        assert(!ttfx_state_reaction_active(&state));
        ttfx_state_destroy(&state);
}

static void test_reaction_visual_is_horizontal_bounded_red_and_finite(void)
{
        ttfx_state_t state = ttfx_state_initial();
        char prompt[] = "Password";

        assert(TTFX_REACTION_TICKS == 168U);
        assert(TTFX_FAILURE_CANDIDATE_TICKS == 1200U);

        assert(ttfx_state_set_password(&state, prompt, 5));
        ttfx_state_set_normal(&state);
        prompt[0] = 'X';
        assert(ttfx_state_set_password(&state, "Password", 0));
        for (unsigned int tick = 0U; tick < 24U; tick++) {
                assert(ttfx_state_reaction_x_offset(&state) == -8);
                ttfx_state_tick(&state);
        }
        assert(ttfx_state_reaction_x_offset(&state) == 6);
        state.reaction_tick = 0U;
        for (unsigned int tick = 0U; tick < TTFX_REACTION_TICKS; tick++) {
                int offset = ttfx_state_reaction_x_offset(&state);
                assert(offset >= -8 && offset <= 8);
                assert(ttfx_state_reaction_row_red(&state, tick % TTFX_GRID_ROWS));
                ttfx_state_tick(&state);
        }
        assert(!ttfx_state_reaction_active(&state));
        assert(ttfx_state_reaction_x_offset(&state) == 0);
        assert(!ttfx_state_reaction_row_red(&state, 0U));

        assert(ttfx_state_set_password(&state, "Password", 2));
        ttfx_state_set_normal(&state);
        assert(ttfx_state_set_password(&state, "Password", 0));
        assert(ttfx_state_reaction_active(&state));
        assert(ttfx_state_set_question(&state, "Recovery key", ""));
        assert(!ttfx_state_reaction_active(&state));
        ttfx_state_destroy(&state);
}

static void test_prompt_modes_and_bullets(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_password(&state, "Password", 7));
        assert(state.prompt_mode == TTFX_PROMPT_PASSWORD);
        assert(state.bullets == 7);

        assert(ttfx_state_set_password(&state, "Password", -3));
        assert(state.bullets == 0);

        assert(ttfx_state_set_question(&state, "Question", "answer"));
        assert(state.prompt_mode == TTFX_PROMPT_QUESTION);
        assert(state.bullets == 0);

        ttfx_state_set_normal(&state);
        assert(state.prompt_mode == TTFX_PROMPT_NONE);
        ttfx_state_destroy(&state);
}

static void test_active_prompt_state_is_owned_for_new_views(void)
{
        ttfx_state_t state = ttfx_state_initial();
        char prompt[] = "Question";
        char answer[] = "visible answer";

        assert(ttfx_state_set_question(&state, prompt, answer));
        prompt[0] = 'X';
        answer[0] = 'X';
        assert(strcmp(state.prompt, "Question") == 0);
        assert(strcmp(state.entry_text, "visible answer") == 0);

        assert(ttfx_state_set_password(&state, "Password", 4));
        assert(strcmp(state.prompt, "Password") == 0);
        assert(state.entry_text == NULL);
        assert(state.bullets == 4);
        ttfx_state_destroy(&state);
}

static void test_messages_restore_latest_still_active_message(void)
{
        ttfx_state_t state = ttfx_state_initial();
        char first[] = "A";

        assert(ttfx_state_set_message(&state, first));
        first[0] = 'X';
        assert(ttfx_state_set_message(&state, "B"));
        assert(strcmp(state.message, "B") == 0);

        assert(ttfx_state_hide_message(&state, "A"));
        assert(state.message_visible);
        assert(strcmp(state.message, "B") == 0);

        assert(ttfx_state_hide_message(&state, "B"));
        assert(!state.message_visible);
        assert(state.message == NULL);
        ttfx_state_destroy(&state);
}

static void test_duplicate_and_unknown_messages_are_safe(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(ttfx_state_set_message(&state, "A"));
        assert(ttfx_state_set_message(&state, "B"));
        assert(ttfx_state_set_message(&state, "A"));
        assert(strcmp(state.message, "A") == 0);

        assert(ttfx_state_hide_message(&state, "A"));
        assert(strcmp(state.message, "B") == 0);
        assert(!ttfx_state_hide_message(&state, "unknown"));
        assert(strcmp(state.message, "B") == 0);
        assert(!ttfx_state_hide_message(&state, NULL));
        assert(strcmp(state.message, "B") == 0);

        assert(ttfx_state_hide_message(&state, "A"));
        assert(strcmp(state.message, "B") == 0);
        assert(!ttfx_state_hide_message(&state, "A"));
        assert(ttfx_state_hide_message(&state, "B"));
        assert(state.message == NULL);
        ttfx_state_destroy(&state);
}

static void test_repeated_password_only_shows_on_hidden_transition(void)
{
        ttfx_view_state_t view = ttfx_view_state_initial();

        assert(ttfx_view_show_prompt(&view));
        assert(!ttfx_view_show_prompt(&view));
        assert(ttfx_view_hide_prompt(&view));
        assert(!ttfx_view_hide_prompt(&view));
        assert(ttfx_view_show_prompt(&view));

        assert(ttfx_view_show_message(&view));
        assert(!ttfx_view_show_message(&view));
        assert(ttfx_view_hide_message(&view));
        assert(!ttfx_view_hide_message(&view));
}

static void test_live_loop_detach_is_idempotent(void)
{
        ttfx_loop_state_t loop = ttfx_loop_state_initial();

        ttfx_loop_watch_attached(&loop);
        assert(ttfx_loop_detach_watch(&loop));
        assert(!ttfx_loop_detach_watch(&loop));
}

static void test_widget_load_failure_rejects_graphical_splash(void)
{
        ttfx_load_state_t load = ttfx_load_state_initial();

        ttfx_load_state_record_view(&load, true);
        assert(ttfx_load_state_can_show(&load));
        ttfx_load_state_record_view(&load, false);
        assert(!ttfx_load_state_can_show(&load));
        ttfx_load_state_record_view(&load, true);
        assert(!ttfx_load_state_can_show(&load));
}

static void test_dynamic_view_failure_is_discarded_without_poisoning_splash(void)
{
        ttfx_load_state_t load = ttfx_load_state_initial();

        ttfx_load_state_record_view(&load, true);
        assert(ttfx_dynamic_view_should_keep(true));
        assert(!ttfx_dynamic_view_should_keep(false));
        assert(ttfx_load_state_can_show(&load));
}

static void test_animation_unavailable_selects_static_only_mode(void)
{
        ttfx_state_t state = ttfx_state_initial();

        assert(state.animation_enabled);
        assert(!state.rendering_failed);
        ttfx_state_disable_animation(&state);
        assert(!state.animation_enabled);
        assert(!state.rendering_failed);
        ttfx_state_destroy(&state);
}

static void test_runtime_label_failure_degrades_without_hiding_prompt(void)
{
        ttfx_state_t state = ttfx_state_initial();
        ttfx_view_state_t view = ttfx_view_state_initial();

        assert(ttfx_view_show_prompt(&view));
        assert(view.prompt_visible);
        assert(ttfx_view_prompt_can_show(&view));
        ttfx_view_degrade_prompt(&state, &view);
        assert(!ttfx_view_prompt_can_show(&view));
        assert(view.prompt_visible);
        assert(!state.animation_enabled);
        assert(!state.rendering_failed);

        /* A repeated callback must keep the entry visible and not retry the label. */
        assert(!ttfx_view_show_prompt(&view));
        ttfx_view_degrade_prompt(&state, &view);
        assert(view.prompt_visible);
        assert(!ttfx_view_prompt_can_show(&view));
        assert(!state.rendering_failed);

        assert(ttfx_view_message_can_show(&view));
        ttfx_view_degrade_message(&state, &view);
        assert(!ttfx_view_message_can_show(&view));
        assert(!view.message_visible);
        assert(!state.animation_enabled);
        assert(!state.rendering_failed);
        ttfx_state_destroy(&state);
}

static void test_geometry_is_centered_and_bounded(void)
{
        ttfx_geometry_t geometry = ttfx_geometry_for_view(1920U, 1080U);

        assert(geometry.columns == TTFX_GRID_COLUMNS);
        assert(geometry.rows == TTFX_GRID_ROWS);
        assert(geometry.cell_width > 0U);
        assert(geometry.cell_height > 0U);
        assert(geometry.cell_width == 5U);
        assert(geometry.grid_width == 826U);
        assert(geometry.grid_height == 200U);
        assert(geometry.cell_height == 10U);
        assert(geometry.grid_x == (1920U - geometry.grid_width) / 2U);
        assert(geometry.grid_y == (1080U - geometry.grid_height) * 9U / 20U);
        assert(geometry.grid_x + geometry.grid_width <= 1920U);
        assert(geometry.grid_y + geometry.grid_height <= 1080U);

        /* Cumulative widths alternate without holes or accumulated pitch error. */
        for (unsigned int column = 0U; column < geometry.columns; column++) {
                unsigned int left = ttfx_geometry_x_edge(&geometry, column);
                unsigned int right = ttfx_geometry_x_edge(&geometry, column + 1U);
                assert(left == column * 51U / 10U);
                assert(right - left == 5U || right - left == 6U);
        }
        assert(ttfx_geometry_x_edge(&geometry, 162U) == geometry.grid_width);
        /* Occupied official wordmark starts one half-row below the canvas. */
        assert(geometry.grid_height - geometry.cell_height == 190U);
        const unsigned int sizes[][2] = {{800U, 600U}, {320U, 200U}, {80U, 40U}, {3U, 2U}, {0U, 0U}};
        for (unsigned int i = 0U; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
                ttfx_geometry_t small = ttfx_geometry_for_view(sizes[i][0], sizes[i][1]);
                assert(small.grid_x + small.grid_width <= sizes[i][0]);
                assert(small.grid_y + small.grid_height <= sizes[i][1]);
                assert(small.grid_width == ttfx_geometry_x_edge(&small, small.columns));
                assert(small.grid_width == small.columns * small.cell_height * 51U / 100U);
        }

        geometry = ttfx_geometry_for_view(80U, 40U);
        assert(geometry.grid_width <= 80U);
        assert(geometry.grid_height <= 40U);

        geometry = ttfx_geometry_for_view(3U, 2U);
        assert(geometry.columns * geometry.cell_width <= geometry.grid_width);
        assert(geometry.rows * geometry.cell_height <= geometry.grid_height);
}

static void test_multiple_views_keep_independent_geometry(void)
{
        ttfx_geometry_t wide = ttfx_geometry_for_view(1920U, 1080U);
        ttfx_geometry_t compact = ttfx_geometry_for_view(800U, 600U);

        assert(wide.grid_x != compact.grid_x);
        assert(wide.grid_y != compact.grid_y);
        assert(wide.grid_x == (1920U - wide.grid_width) / 2U);
        assert(compact.grid_x == (800U - compact.grid_width) / 2U);
}


int main(void)
{
        test_tick_progresses_and_wraps();
        test_input_only_freezes_after_first_natural_completion();
        test_input_only_fast_forwards_when_success_precedes_completion();
        test_continuous_mode_preserves_current_looping_behavior();
        test_retry_returns_submit_to_finish_mode_to_input();
        test_submit_accelerates_to_final_for_verdict();
        test_failed_answer_mid_fast_forward_loops_after_final();
        test_empty_submit_does_not_accelerate();
        test_failed_nonempty_password_replacement_triggers_once();
        test_password_candidate_expires_to_normal();
        test_clear_prompt_never_arms_password_failure();
        test_password_failure_sequence_rejects_false_positives();
        test_reaction_visual_is_horizontal_bounded_red_and_finite();
        test_prompt_modes_and_bullets();
        test_active_prompt_state_is_owned_for_new_views();
        test_messages_restore_latest_still_active_message();
        test_duplicate_and_unknown_messages_are_safe();
        test_repeated_password_only_shows_on_hidden_transition();
        test_live_loop_detach_is_idempotent();
        test_widget_load_failure_rejects_graphical_splash();
        test_dynamic_view_failure_is_discarded_without_poisoning_splash();
        test_animation_unavailable_selects_static_only_mode();
        test_runtime_label_failure_degrades_without_hiding_prompt();
        test_geometry_is_centered_and_bounded();
        test_multiple_views_keep_independent_geometry();

        puts("state tests: PASS");
        return 0;
}
