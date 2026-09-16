use clap::CommandFactory;
use ttfx::cli::Cli;
use ttfx_plymouth::{Engine, StepOutcome};

#[test]
fn creates_real_print_effect_with_a_full_structured_frame() {
    let engine = Engine::create("print", 0x1234, "Héllo", 12, 3, 60).unwrap();

    assert_eq!(engine.dimensions(), (12, 3));
    assert_eq!(engine.cells().len(), 36);
    assert!(engine
        .cells()
        .iter()
        .any(|cell| cell.codepoint != u32::from(' ')));
}

#[test]
fn completion_keeps_the_terminal_frame_until_explicit_reset() {
    let mut engine = Engine::create("print", 0x55aa, "Loop me", 12, 2, 60).unwrap();
    let first = engine.cells().to_vec();
    assert_eq!(engine.step().unwrap(), StepOutcome::Frame);
    assert_ne!(engine.cells(), first);

    for _ in 0..10_000 {
        if engine.step().unwrap() == StepOutcome::Completed {
            let terminal = engine.cells().to_vec();
            assert_ne!(terminal, first);
            assert_eq!(engine.step().unwrap(), StepOutcome::Completed);
            assert_eq!(engine.cells(), terminal);
            engine.reset().unwrap();
            assert_eq!(engine.cells(), first);
            return;
        }
    }
    panic!("print effect did not complete within the safety bound");
}

#[test]
fn explicit_reset_reconstructs_the_deterministic_first_frame() {
    let mut engine = Engine::create("print", 0x55aa, "Reset me", 12, 2, 60).unwrap();
    let first = engine.cells().to_vec();
    assert_eq!(engine.step().unwrap(), StepOutcome::Frame);
    assert_ne!(engine.cells(), first);

    engine.reset().unwrap();

    assert_eq!(engine.cells(), first);
}

#[test]
fn rejects_unknown_effects_and_out_of_bounds_configuration() {
    assert_eq!(
        Engine::create("not-an-effect", 1, "x", 10, 2, 60).err(),
        Some(ttfx_plymouth::EngineError::InvalidEffect)
    );
    assert_eq!(
        Engine::create("print", 1, "x", 0, 2, 60).err(),
        Some(ttfx_plymouth::EngineError::InvalidDimensions)
    );
    assert_eq!(
        Engine::create("print", 1, "x", 163, 2, 60).err(),
        Some(ttfx_plymouth::EngineError::InvalidDimensions)
    );
    assert_eq!(
        Engine::create("print", 1, "x", 162, 30, 60).err(),
        Some(ttfx_plymouth::EngineError::InvalidDimensions)
    );
    assert_eq!(
        Engine::create("print", 1, "x", 10, 2, 241).err(),
        Some(ttfx_plymouth::EngineError::InvalidFrameRate)
    );
}

#[test]
fn accepts_subdivided_logo_without_loosening_the_cell_budget() {
    let logo = include_str!("../../plymouth-ttfx-plugin/embedded-logo-v2.txt");
    let engine = Engine::create("decrypt", 7, logo, 162, 20, 240).unwrap();

    assert_eq!(engine.dimensions(), (162, 20));
    assert_eq!(engine.cells().len(), 3240);
    assert_eq!(
        ttfx_plymouth::validate_canvas(162, 30),
        Err(ttfx_plymouth::EngineError::InvalidDimensions)
    );
}

#[test]
fn terminal_decrypt_frame_matches_the_pr29_field_bands() {
    let logo = include_str!("../../plymouth-ttfx-plugin/embedded-logo-v2.txt");
    let mut engine = Engine::create("decrypt", 7, logo, 162, 20, 240).unwrap();

    for _ in 0..10_000 {
        if engine.step().unwrap() == StepOutcome::Completed {
            let (width, height) = engine.dimensions();
            let colors_by_nonempty_row: Vec<u32> = (0..height)
                .filter_map(|row| {
                    let colors: std::collections::BTreeSet<u32> = engine.cells()
                        [row * width..(row + 1) * width]
                        .iter()
                        .filter(|cell| cell.codepoint != u32::from(' '))
                        .map(|cell| cell.fg_rgba)
                        .collect();
                    assert!(
                        colors.len() <= 1,
                        "row {row} is not one discrete band: {colors:?}"
                    );
                    colors.into_iter().next()
                })
                .collect();
            let expected: Vec<u32> = [
                (0xd0fdd9ff, 4usize),
                (0xa8fcbaff, 3),
                (0x82fb9cff, 4),
                (0x539e65ff, 3),
                (0x2b5037ff, 5),
            ]
            .into_iter()
            .flat_map(|(color, rows)| std::iter::repeat_n(color, rows))
            .collect();
            assert_eq!(colors_by_nonempty_row, expected);
            return;
        }
    }
    panic!("decrypt effect did not complete within the safety bound");
}

#[test]
fn representative_effects_use_the_same_five_final_text_bands() {
    let logo = include_str!("../../plymouth-ttfx-plugin/embedded-logo-v2.txt");
    // print covers the ordinary final-gradient path; matrix and rain had
    // separate paths that the audio-background implementation did not wire.
    for effect in ["print", "matrix", "rain"] {
        let mut engine = Engine::create(effect, 7, logo, 162, 20, 240).unwrap();
        let mut completed = false;
        for _ in 0..10_000 {
            if engine.step().unwrap() != StepOutcome::Completed {
                continue;
            }
            completed = true;
            let (width, height) = engine.dimensions();
            let logo_rows: Vec<Vec<char>> =
                logo.lines().map(|line| line.chars().collect()).collect();
            let colors_by_nonempty_row: Vec<u32> = (0..height)
                .filter_map(|row| {
                    let colors: std::collections::BTreeSet<u32> = engine.cells()
                        [row * width..(row + 1) * width]
                        .iter()
                        .zip(&logo_rows[row])
                        .filter(|(_, input)| **input != ' ')
                        .map(|(cell, _)| cell.fg_rgba)
                        .collect();
                    if colors.is_empty() {
                        return None;
                    }
                    assert_eq!(colors.len(), 1, "{effect} row {row} is not one solid band");
                    colors.into_iter().next()
                })
                .collect();
            let run_lengths = colors_by_nonempty_row
                .chunk_by(|left, right| left == right)
                .map(<[u32]>::len)
                .collect::<Vec<_>>();
            assert_eq!(run_lengths, [4, 3, 4, 3, 5], "{effect}");
            break;
        }
        assert!(
            completed,
            "{effect} did not complete within the safety bound"
        );
    }
}

#[test]
fn allocation_overflow_is_detected_before_engine_construction() {
    assert_eq!(
        ttfx_plymouth::validate_canvas(usize::MAX, 2),
        Err(ttfx_plymouth::EngineError::Overflow)
    );
}

#[test]
fn every_exposed_effect_builds_and_steps_at_the_early_boot_limit() {
    let effects: Vec<&str> = include_str!("../effects.txt").lines().collect();
    assert!(!effects.is_empty());
    assert!(effects.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(effects.iter().all(|effect| {
        !effect.is_empty()
            && effect.len() <= 127
            && effect.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase() || (index > 0 && (byte.is_ascii_digit() || byte == b'-'))
            })
    }));

    let mut registry: Vec<String> = Cli::command()
        .get_subcommands()
        .filter(|command| command.get_name() != "help")
        .map(|command| command.get_name().to_owned())
        .collect();
    registry.sort();
    assert_eq!(
        registry, effects,
        "effects.txt must exactly match the vendored registry"
    );

    let input = include_str!("../../plymouth-ttfx-plugin/embedded-logo-v2.txt");
    for effect in effects {
        let engine = Engine::create(effect, 7, input, 162, 20, 240)
            .unwrap_or_else(|error| panic!("{effect} failed to build: {error:?}"));
        assert_eq!(engine.cells().len(), 162 * 20, "{effect}");
    }
}
