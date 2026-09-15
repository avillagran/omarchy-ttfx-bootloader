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
fn completion_restarts_with_the_same_seed_and_first_frame() {
    let mut engine = Engine::create("print", 0x55aa, "Loop me", 12, 2, 60).unwrap();
    let first = engine.cells().to_vec();
    assert_eq!(engine.step().unwrap(), StepOutcome::Frame);
    assert_ne!(engine.cells(), first);

    for _ in 0..10_000 {
        if engine.step().unwrap() == StepOutcome::Looped {
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
