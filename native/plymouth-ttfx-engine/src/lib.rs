use clap::Parser;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use ttfx::cli::Cli;
use ttfx::engine::animation::CharacterVisual;
use ttfx::engine::ctx::{Clock, EngineCtx};
use ttfx::engine::effect::Effect;
use ttfx::utils::rng::Rng;

pub const CELL_FLAG_FG: u32 = 1 << 0;
pub const CELL_FLAG_BG: u32 = 1 << 1;
pub const CELL_FLAG_BOLD: u32 = 1 << 2;
pub const CELL_FLAG_DIM: u32 = 1 << 3;
pub const CELL_FLAG_ITALIC: u32 = 1 << 4;
pub const CELL_FLAG_UNDERLINE: u32 = 1 << 5;
pub const CELL_FLAG_BLINK: u32 = 1 << 6;
pub const CELL_FLAG_REVERSE: u32 = 1 << 7;
pub const CELL_FLAG_HIDDEN: u32 = 1 << 8;
pub const CELL_FLAG_STRIKE: u32 = 1 << 9;
pub const MAX_WIDTH: usize = 162;
pub const MAX_HEIGHT: usize = 40;
pub const MAX_CELLS: usize = 4_800;
pub const MAX_FPS: u32 = 240;
pub const MAX_EFFECT_NAME_BYTES: usize = 128;
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const TTFX_STATUS_OK: i32 = 0;
pub const TTFX_STATUS_INVALID_ARGUMENT: i32 = 1;
pub const TTFX_STATUS_INVALID_EFFECT: i32 = 2;
pub const TTFX_STATUS_INVALID_DIMENSIONS: i32 = 3;
pub const TTFX_STATUS_INVALID_FRAME_RATE: i32 = 4;
pub const TTFX_STATUS_INVALID_UTF8: i32 = 5;
pub const TTFX_STATUS_OVERFLOW: i32 = 6;
pub const TTFX_STATUS_ENGINE_ERROR: i32 = 7;
pub const TTFX_STATUS_PANIC: i32 = 8;
pub const TTFX_STATUS_INPUT_TOO_LARGE: i32 = 9;
pub const TTFX_STATUS_POISONED: i32 = 10;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub codepoint: u32,
    pub fg_rgba: u32,
    pub bg_rgba: u32,
    pub flags: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EngineError {
    InvalidEffect,
    InvalidDimensions,
    InvalidFrameRate,
    Overflow,
    Build,
    InputTooLarge,
}

pub fn validate_canvas(width: usize, height: usize) -> Result<usize, EngineError> {
    let cells = width.checked_mul(height).ok_or(EngineError::Overflow)?;
    if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT || cells > MAX_CELLS {
        return Err(EngineError::InvalidDimensions);
    }
    Ok(cells)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    Frame = 0,
    Completed = 1,
}

struct Parameters {
    effect_name: String,
    seed: u64,
    input: String,
    width: u32,
    height: u32,
    fps: u32,
    background_rgb: u32,
}

pub struct Engine {
    parameters: Parameters,
    effect: Box<dyn Effect>,
    ctx: EngineCtx,
    cells: Vec<Cell>,
    width: usize,
    height: usize,
    completed: bool,
}

/// Opaque C handle. It must be used and freed on the creating thread.
pub struct TtfxEngine {
    inner: Engine,
    poisoned: bool,
}

/// Playback mode for [`Engine::draw_logo`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogoMode {
    /// Plays the animation endlessly, seamlessly restarting at the end.
    Loop,
    /// Plays the animation exactly once. Once playback completes, the final
    /// frame stays exposed through `cells()` and the outcome reports
    /// `StepOutcome::Completed` on every subsequent call.
    OneTime,
}

impl Engine {
    pub fn create(
        effect_name: &str,
        seed: u64,
        input: &str,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Self, EngineError> {
        Self::create_with_background(effect_name, seed, input, width, height, fps, 0x000000)
    }

    pub fn create_with_background(
        effect_name: &str,
        seed: u64,
        input: &str,
        width: u32,
        height: u32,
        fps: u32,
        background_rgb: u32,
    ) -> Result<Self, EngineError> {
        if input.len() > MAX_INPUT_BYTES {
            return Err(EngineError::InputTooLarge);
        }
        if fps == 0 || fps > MAX_FPS {
            return Err(EngineError::InvalidFrameRate);
        }
        let cell_count = validate_canvas(width as usize, height as usize)?;
        let parameters = Parameters {
            effect_name: effect_name.to_owned(),
            seed,
            input: input.to_owned(),
            width,
            height,
            fps,
            background_rgb,
        };
        let (effect, ctx) = build_instance(&parameters)?;
        let mut engine = Self {
            parameters,
            effect,
            ctx,
            cells: Vec::with_capacity(cell_count),
            width: width as usize,
            height: height as usize,
            completed: false,
        };
        engine
            .effect
            .next_frame(&mut engine.ctx)
            .ok_or(EngineError::Build)?;
        engine.snapshot();
        Ok(engine)
    }

    pub fn step(&mut self) -> Result<StepOutcome, EngineError> {
        if self.completed {
            return Ok(StepOutcome::Completed);
        }
        if self.effect.next_frame(&mut self.ctx).is_some() {
            self.snapshot();
            Ok(StepOutcome::Frame)
        } else {
            self.completed = true;
            Ok(StepOutcome::Completed)
        }
    }

    /// Draws the logo according to `mode`. Every caller keeps its playback
    /// state exclusively through this function; the current frame is always
    /// available through `cells()` after the call.
    ///
    /// - [`LogoMode::Loop`]: advances the animation and, exactly at the tape
    ///   boundary, reconstructs the next cycle deterministically. Returns
    ///   `StepOutcome::Completed` only for the call that wrapped, so callers
    ///   can count cycles.
    /// - [`LogoMode::OneTime`]: advances the animation until it completes and
    ///   then holds the final frame; returns `StepOutcome::Completed` once
    ///   playback has finished and on every later call.
    pub fn draw_logo(&mut self, mode: LogoMode) -> Result<StepOutcome, EngineError> {
        match mode {
            LogoMode::OneTime => self.step(),
            LogoMode::Loop => match self.step()? {
                StepOutcome::Completed => {
                    self.reset()?;
                    Ok(StepOutcome::Completed)
                }
                StepOutcome::Frame => Ok(StepOutcome::Frame),
            },
        }
    }

    pub fn reset(&mut self) -> Result<(), EngineError> {
        self.restart()?;
        self.snapshot();
        self.completed = false;
        Ok(())
    }

    fn restart(&mut self) -> Result<(), EngineError> {
        let (effect, ctx) = build_instance(&self.parameters)?;
        self.effect = effect;
        self.ctx = ctx;
        self.effect
            .next_frame(&mut self.ctx)
            .ok_or(EngineError::Build)?;
        Ok(())
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    fn snapshot(&mut self) {
        let mut bottom_up = Vec::with_capacity(self.width * self.height);
        self.ctx
            .terminal
            .visit_render_cells(|visual| bottom_up.push(cell_from_visual(visual)));
        self.cells = top_to_bottom(&bottom_up, self.width, self.height);
    }
}

fn top_to_bottom<T: Clone>(bottom_up: &[T], width: usize, height: usize) -> Vec<T> {
    debug_assert_eq!(bottom_up.len(), width * height);
    let mut result = Vec::with_capacity(bottom_up.len());
    for row in (0..height).rev() {
        result.extend_from_slice(&bottom_up[row * width..(row + 1) * width]);
    }
    result
}

fn build_instance(parameters: &Parameters) -> Result<(Box<dyn Effect>, EngineCtx), EngineError> {
    let mut arguments = vec!["ttfx", parameters.effect_name.as_str()];
    if parameters.effect_name == "decrypt" {
        arguments.extend([
            "--final-gradient-stops",
            // Canonical Omarchy field colors, top to bottom. The vendored
            // final-text mapper applies the shared 4:3:4:3:5 row bands.
            "d0fdd9",
            "a8fcba",
            "82fb9c",
            "539e65",
            "2b5037",
            "--final-gradient-steps",
            "1",
            "1",
            "1",
            "1",
            "--final-gradient-direction",
            "vertical",
        ]);
    }
    let cli = Cli::try_parse_from(arguments).map_err(|_| EngineError::InvalidEffect)?;
    let mut config = cli.terminal_config();
    let command = cli.effect.ok_or(EngineError::InvalidEffect)?;
    config.canvas_width = i64::from(parameters.width);
    config.canvas_height = i64::from(parameters.height);
    config.frame_rate = i64::from(parameters.fps);
    config.ignore_terminal_dimensions = true;
    config.terminal_background_color =
        ttfx::utils::graphics::Color::from_hex(&format!("{:06x}", parameters.background_rgb))
            .map_err(|_| EngineError::Build)?;
    let mut ctx = EngineCtx::new(
        &parameters.input,
        config,
        Rng::seeded(parameters.seed),
        Clock::virtual_with_frame_rate(i64::from(parameters.fps)),
    )
    .map_err(|_| EngineError::Build)?;
    ctx.set_structured_output(true);
    ctx.final_text_bands = true;
    let mut effect = command.build_effect();
    effect.build(&mut ctx).map_err(|_| EngineError::Build)?;
    Ok((effect, ctx))
}

fn cell_from_visual(visual: Option<&CharacterVisual>) -> Cell {
    let Some(visual) = visual else {
        return Cell {
            codepoint: u32::from(' '),
            fg_rgba: 0,
            bg_rgba: 0,
            flags: 0,
        };
    };
    let mut flags = 0;
    let (fg_rgba, bg_rgba) = visual.colors.map_or((0, 0), |colors| {
        let fg = colors.fg_color.map_or(0, |color| {
            flags |= CELL_FLAG_FG;
            rgba(color.rgb_ints())
        });
        let bg = colors.bg_color.map_or(0, |color| {
            flags |= CELL_FLAG_BG;
            rgba(color.rgb_ints())
        });
        (fg, bg)
    });
    flags |= u32::from(visual.bold) * CELL_FLAG_BOLD;
    flags |= u32::from(visual.dim) * CELL_FLAG_DIM;
    flags |= u32::from(visual.italic) * CELL_FLAG_ITALIC;
    flags |= u32::from(visual.underline) * CELL_FLAG_UNDERLINE;
    flags |= u32::from(visual.blink) * CELL_FLAG_BLINK;
    flags |= u32::from(visual.reverse) * CELL_FLAG_REVERSE;
    flags |= u32::from(visual.hidden) * CELL_FLAG_HIDDEN;
    flags |= u32::from(visual.strike) * CELL_FLAG_STRIKE;
    Cell {
        codepoint: visual
            .symbol
            .chars()
            .next()
            .map_or(u32::from(' '), u32::from),
        fg_rgba,
        bg_rgba,
        flags,
    }
}

fn rgba((r, g, b): (u8, u8, u8)) -> u32 {
    (u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | 0xff
}

fn status(error: EngineError) -> i32 {
    match error {
        EngineError::InvalidEffect => TTFX_STATUS_INVALID_EFFECT,
        EngineError::InvalidDimensions => TTFX_STATUS_INVALID_DIMENSIONS,
        EngineError::InvalidFrameRate => TTFX_STATUS_INVALID_FRAME_RATE,
        EngineError::Overflow => TTFX_STATUS_OVERFLOW,
        EngineError::Build => TTFX_STATUS_ENGINE_ERROR,
        EngineError::InputTooLarge => TTFX_STATUS_INPUT_TOO_LARGE,
    }
}

fn ffi_boundary(operation: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(TTFX_STATUS_PANIC)
}

unsafe fn ffi_mutating_boundary(
    engine: *mut TtfxEngine,
    operation: impl FnOnce(&mut TtfxEngine) -> i32,
) -> i32 {
    if engine.is_null() {
        return TTFX_STATUS_INVALID_ARGUMENT;
    }
    let engine = unsafe { &mut *engine };
    if engine.poisoned {
        return TTFX_STATUS_POISONED;
    }
    match catch_unwind(AssertUnwindSafe(|| operation(engine))) {
        Ok(status) => status,
        Err(_) => {
            engine.poisoned = true;
            TTFX_STATUS_PANIC
        }
    }
}

unsafe fn bounded_c_str<'a>(value: *const std::ffi::c_char) -> Result<&'a str, i32> {
    if value.is_null() {
        return Err(TTFX_STATUS_INVALID_ARGUMENT);
    }
    let mut length = None;
    for index in 0..MAX_EFFECT_NAME_BYTES {
        if unsafe { *value.add(index) } == 0 {
            length = Some(index);
            break;
        }
    }
    let length = length.ok_or(TTFX_STATUS_INVALID_ARGUMENT)?;
    let bytes = unsafe { std::slice::from_raw_parts(value.cast::<u8>(), length) };
    std::str::from_utf8(bytes).map_err(|_| TTFX_STATUS_INVALID_UTF8)
}

/// Create an engine and expose its deterministic first frame. `input` is a byte
/// buffer. `effect` must point to memory readable through a NUL byte within
/// `MAX_EFFECT_NAME_BYTES` bytes.
/// On every failure `*out_engine` is set to null.
///
/// # Safety
///
/// All non-null pointers must be valid for the documented reads or writes.
/// The returned handle must be used only on its creating thread and freed once.
#[no_mangle]
pub unsafe extern "C" fn ttfx_engine_create(
    effect: *const std::ffi::c_char,
    seed: u64,
    input: *const u8,
    input_len: usize,
    width: u32,
    height: u32,
    fps: u32,
    out_engine: *mut *mut TtfxEngine,
) -> i32 {
    unsafe {
        ttfx_engine_create_with_background(
            effect, seed, input, input_len, width, height, fps, 0x000000, out_engine,
        )
    }
}

/// Create an embedded engine whose terminal background matches its host surface.
/// `background_rgb` is a 24-bit `0xRRGGBB` color.
///
/// # Safety
///
/// The pointer requirements are identical to [`ttfx_engine_create`].
#[no_mangle]
pub unsafe extern "C" fn ttfx_engine_create_with_background(
    effect: *const std::ffi::c_char,
    seed: u64,
    input: *const u8,
    input_len: usize,
    width: u32,
    height: u32,
    fps: u32,
    background_rgb: u32,
    out_engine: *mut *mut TtfxEngine,
) -> i32 {
    ffi_boundary(|| {
        if out_engine.is_null() {
            return TTFX_STATUS_INVALID_ARGUMENT;
        }
        unsafe { *out_engine = ptr::null_mut() };
        if input_len > MAX_INPUT_BYTES {
            return TTFX_STATUS_INPUT_TOO_LARGE;
        }
        if input.is_null() && input_len != 0 {
            return TTFX_STATUS_INVALID_ARGUMENT;
        }
        let effect = match unsafe { bounded_c_str(effect) } {
            Ok(effect) => effect,
            Err(error) => return error,
        };
        let input_bytes = if input_len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(input, input_len) }
        };
        let input = match std::str::from_utf8(input_bytes) {
            Ok(input) => input,
            Err(_) => return TTFX_STATUS_INVALID_UTF8,
        };
        match Engine::create_with_background(
            effect,
            seed,
            input,
            width,
            height,
            fps,
            background_rgb,
        ) {
            Ok(inner) => {
                unsafe {
                    *out_engine = Box::into_raw(Box::new(TtfxEngine {
                        inner,
                        poisoned: false,
                    }))
                };
                TTFX_STATUS_OK
            }
            Err(error) => status(error),
        }
    })
}

/// Advance one live frame. `*out_looped` is 1 when this is the first frame of
/// a deterministically reconstructed loop.
///
/// # Safety
///
/// `engine` must be a live handle created by this library on the current thread,
/// and `out_looped` must be valid for one byte of writable memory.
#[no_mangle]
pub unsafe extern "C" fn ttfx_engine_step(engine: *mut TtfxEngine, out_looped: *mut u8) -> i32 {
    if out_looped.is_null() {
        return TTFX_STATUS_INVALID_ARGUMENT;
    }
    unsafe { *out_looped = 0 };
    unsafe {
        ffi_mutating_boundary(engine, |engine| match engine.inner.step() {
            Ok(outcome) => {
                *out_looped = u8::from(outcome == StepOutcome::Completed);
                TTFX_STATUS_OK
            }
            Err(error) => status(error),
        })
    }
}

/// Reconstruct the effect from its original parameters and expose its first frame.
///
/// # Safety
///
/// `engine` must be a live handle created by this library on the current thread.
#[no_mangle]
pub unsafe extern "C" fn ttfx_engine_reset(engine: *mut TtfxEngine) -> i32 {
    unsafe {
        ffi_mutating_boundary(engine, |engine| match engine.inner.reset() {
            Ok(()) => TTFX_STATUS_OK,
            Err(error) => status(error),
        })
    }
}

/// Draws the logo according to `mode` (TTFX_LOGO_MODE_*). Every caller keeps
/// its playback state exclusively through this function; the current frame is
/// always available through `ttfx_engine_cells` after the call.
///
/// `out_looped` (may be NULL) is set to 1 only for the call in which a
/// TTFX_LOGO_MODE_LOOP playback wrapped to a fresh cycle. `out_finished`
/// (may be NULL) is set to 1 once a TTFX_LOGO_MODE_ONE_TIME playback has
/// completed and the final frame is being held; it stays 1 on later calls.
///
/// # Safety
///
/// `engine` must be a live handle created by this library on the current
/// thread, and each output pointer must be valid for one byte when non-NULL.
#[no_mangle]
pub unsafe extern "C" fn ttfx_engine_draw_logo(
    engine: *mut TtfxEngine,
    mode: u32,
    out_looped: *mut u8,
    out_finished: *mut u8,
) -> i32 {
    if !out_looped.is_null() {
        unsafe { *out_looped = 0 };
    }
    if !out_finished.is_null() {
        unsafe { *out_finished = 0 };
    }
    let mode = match mode {
        x if x == LogoMode::Loop as u32 => LogoMode::Loop,
        x if x == LogoMode::OneTime as u32 => LogoMode::OneTime,
        _ => return TTFX_STATUS_INVALID_ARGUMENT,
    };
    unsafe {
        ffi_mutating_boundary(engine, |engine| match engine.inner.draw_logo(mode) {
            Ok(outcome) => {
                match mode {
                    LogoMode::Loop => {
                        if !out_looped.is_null() {
                            *out_looped = u8::from(outcome == StepOutcome::Completed);
                        }
                    }
                    LogoMode::OneTime => {
                        if !out_finished.is_null() {
                            *out_finished = u8::from(outcome == StepOutcome::Completed);
                        }
                    }
                }
                TTFX_STATUS_OK
            }
            Err(error) => status(error),
        })
    }
}

/// Borrow the top-to-bottom row-major cell array. The pointer remains valid
/// until the next mutable call on this handle or until the handle is freed.
///
/// # Safety
///
/// `engine` must be a live handle, and every output pointer must be writable.
#[no_mangle]
pub unsafe extern "C" fn ttfx_engine_cells(
    engine: *const TtfxEngine,
    out_cells: *mut *const Cell,
    out_count: *mut usize,
    out_width: *mut u32,
    out_height: *mut u32,
) -> i32 {
    ffi_boundary(|| {
        if !out_cells.is_null() {
            unsafe { *out_cells = ptr::null() };
        }
        if !out_count.is_null() {
            unsafe { *out_count = 0 };
        }
        if !out_width.is_null() {
            unsafe { *out_width = 0 };
        }
        if !out_height.is_null() {
            unsafe { *out_height = 0 };
        }
        if engine.is_null()
            || out_cells.is_null()
            || out_count.is_null()
            || out_width.is_null()
            || out_height.is_null()
        {
            return TTFX_STATUS_INVALID_ARGUMENT;
        }
        let handle = unsafe { &*engine };
        if handle.poisoned {
            return TTFX_STATUS_POISONED;
        }
        let engine = &handle.inner;
        unsafe {
            *out_cells = engine.cells().as_ptr();
            *out_count = engine.cells().len();
            *out_width = engine.width as u32;
            *out_height = engine.height as u32;
        }
        TTFX_STATUS_OK
    })
}

/// Free a handle. Passing null is a no-op.
///
/// # Safety
///
/// A non-null `engine` must be a live handle returned by this library, on its
/// creating thread, and must not have been freed previously.
#[no_mangle]
pub unsafe extern "C" fn ttfx_engine_free(engine: *mut TtfxEngine) {
    if !engine.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe { drop(Box::from_raw(engine)) }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ttfx::engine::animation::VisualParams;
    use ttfx::utils::graphics::{Color, ColorPair};

    #[test]
    fn row_conversion_covers_full_canvas_top_to_bottom() {
        let bottom_up = [1, 2, 3, 4, 5, 6];
        assert_eq!(top_to_bottom(&bottom_up, 2, 3), [5, 6, 3, 4, 1, 2]);
    }

    #[test]
    fn visual_conversion_preserves_unicode_rgba_and_all_flags() {
        let visual = CharacterVisual::new(
            "λtail",
            VisualParams {
                bold: true,
                dim: true,
                italic: true,
                underline: true,
                blink: true,
                reverse: true,
                hidden: true,
                strike: true,
                colors: Some(ColorPair::new(
                    Some(Color::from_hex("123456").unwrap()),
                    Some(Color::from_hex("abcdef").unwrap()),
                )),
                ..VisualParams::default()
            },
        );

        assert_eq!(
            cell_from_visual(Some(&visual)),
            Cell {
                codepoint: u32::from('λ'),
                fg_rgba: 0x123456ff,
                bg_rgba: 0xabcdefff,
                flags: CELL_FLAG_FG
                    | CELL_FLAG_BG
                    | CELL_FLAG_BOLD
                    | CELL_FLAG_DIM
                    | CELL_FLAG_ITALIC
                    | CELL_FLAG_UNDERLINE
                    | CELL_FLAG_BLINK
                    | CELL_FLAG_REVERSE
                    | CELL_FLAG_HIDDEN
                    | CELL_FLAG_STRIKE,
            }
        );
        assert_eq!(cell_from_visual(None).codepoint, u32::from(' '));
        assert_eq!(
            cell_from_visual(Some(&CharacterVisual::plain(""))).codepoint,
            u32::from(' ')
        );
    }

    #[test]
    fn embedded_engine_uses_loader_background_color() {
        let engine =
            Engine::create_with_background("print", 7, "OMARCHY", 12, 2, 60, 0x1a1b26).unwrap();

        assert_eq!(
            engine
                .ctx
                .terminal
                .config
                .terminal_background_color
                .rgb_ints(),
            (0x1a, 0x1b, 0x26)
        );
    }

    #[test]
    fn draw_logo_loop_wraps_and_restarts_deterministically() {
        let mut engine = Engine::create("decrypt", 7, "", 162, 20, 240).unwrap();
        let first_frame = engine.cells().to_vec();

        let mut wrapped_at = None;
        for call in 1..=4096 {
            match engine.draw_logo(LogoMode::Loop) {
                Ok(StepOutcome::Completed) => {
                    wrapped_at = Some(call);
                    break;
                }
                Ok(StepOutcome::Frame) => {}
                Err(error) => panic!("loop draw failed: {error:?}"),
            }
        }
        let wrapped_at = wrapped_at.expect("decrypt must wrap within 4096 draws");
        assert_eq!(
            engine.cells().to_vec(),
            first_frame,
            "wrap must restart the tape"
        );

        // The loop keeps producing frames afterwards without a manual reset.
        assert!(matches!(
            engine.draw_logo(LogoMode::Loop),
            Ok(StepOutcome::Frame)
        ));
        assert!(wrapped_at > 0);
    }

    #[test]
    fn draw_logo_one_time_finishes_and_holds_the_final_frame() {
        let mut engine = Engine::create("decrypt", 7, "", 162, 20, 240).unwrap();

        let mut finished_at = None;
        for call in 1..=4096 {
            match engine.draw_logo(LogoMode::OneTime) {
                Ok(StepOutcome::Completed) => {
                    finished_at = Some(call);
                    break;
                }
                Ok(StepOutcome::Frame) => {}
                Err(error) => panic!("one_time draw failed: {error:?}"),
            }
        }
        assert!(
            finished_at.is_some(),
            "decrypt must finish within 4096 draws"
        );

        let final_frame = engine.cells().to_vec();
        // Once finished, every later call reports completion and keeps the
        // exact final frame: one_time holds the last frame forever.
        for _ in 0..8 {
            assert!(matches!(
                engine.draw_logo(LogoMode::OneTime),
                Ok(StepOutcome::Completed)
            ));
            assert_eq!(engine.cells().to_vec(), final_frame);
        }
        assert_ne!(
            final_frame,
            Engine::create("decrypt", 7, "", 162, 20, 240)
                .unwrap()
                .cells()
                .to_vec()
        );
    }

    #[test]
    fn draw_logo_mode_flag_reports_through_ffi() {
        let inner = Engine::create("decrypt", 7, "", 162, 20, 240).unwrap();
        let mut handle = TtfxEngine {
            inner,
            poisoned: false,
        };
        let raw = &mut handle as *mut TtfxEngine;

        assert_eq!(
            unsafe { ttfx_engine_draw_logo(raw, 2, ptr::null_mut(), ptr::null_mut()) },
            TTFX_STATUS_INVALID_ARGUMENT
        );

        let mut looped = 0u8;
        let mut finished = 0u8;
        assert_eq!(
            unsafe { ttfx_engine_draw_logo(raw, 0, &mut looped, &mut finished) },
            TTFX_STATUS_OK
        );
        assert_eq!(looped, 0);
        assert_eq!(finished, 0);

        assert_eq!(
            unsafe { ttfx_engine_draw_logo(raw, 1, &mut looped, &mut finished) },
            TTFX_STATUS_OK
        );
        assert_eq!(finished, 0, "decrypt does not finish in two draws");
    }

    #[test]
    fn panic_boundary_maps_panics_to_status() {
        assert_eq!(
            ffi_boundary(|| panic!("containment probe")),
            TTFX_STATUS_PANIC
        );
    }

    #[test]
    fn mutating_panic_poisons_the_handle_until_it_is_freed() {
        let inner = Engine::create("print", 7, "OMARCHY", 12, 2, 60).unwrap();
        let mut handle = TtfxEngine {
            inner,
            poisoned: false,
        };
        let raw = &mut handle as *mut TtfxEngine;

        assert_eq!(
            unsafe {
                ffi_mutating_boundary(raw, |_engine| {
                    panic!("mutation panic probe");
                })
            },
            TTFX_STATUS_PANIC
        );
        assert!(handle.poisoned);
        assert_eq!(
            unsafe { ffi_mutating_boundary(raw, |_engine| TTFX_STATUS_OK) },
            TTFX_STATUS_POISONED
        );
    }
}
