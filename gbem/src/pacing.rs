//! How many emulated frames to run per repaint.
//!
//! This is the pacing *policy*, kept free of I/O so it can be tested directly.
//! The frontend reads the real clock (audio-queue depth when a device is open,
//! wall-clock otherwise) and hands the signal to [`Pacer::frames_due`]; the
//! pacer answers with a frame count and carries the wall-clock remainder.

use std::time::Duration;

/// DMG frame rate: 4194304 Hz / 70224 cycles per frame ≈ 59.73 Hz.
pub const GB_FPS: f64 = 4_194_304.0 / 70_224.0;

/// Audio headroom to keep buffered, in seconds (~60 ms). Topping up to this
/// depth inherently bounds a single audio top-up to `AUDIO_TARGET_SECS *
/// GB_FPS` ≈ 4 frames, so no extra cap is needed on that path.
const AUDIO_TARGET_SECS: f64 = 0.060;
/// Never run more than this many frames from one wall-clock tick.
const WALL_MAX_FRAMES: u32 = 4;
/// Cap a single elapsed sample so a stall doesn't fast-forward the game.
const MAX_ELAPSED_SECS: f64 = 0.1;

/// The timing signal available to the frontend this repaint.
pub enum Clock {
    /// The audio device is the clock: how many stereo sample-frames are queued.
    Audio { queued_frames: usize },
    /// No audio device: real time elapsed since the previous repaint.
    Wall { elapsed: Duration },
}

pub struct Pacer {
    sample_rate: u32,
    /// Fractional wall-clock frames carried between repaints.
    frame_accum: f64,
}

impl Pacer {
    pub fn new(sample_rate: u32) -> Self {
        Pacer {
            sample_rate: sample_rate.max(1),
            frame_accum: 0.0,
        }
    }

    /// Frames to run this repaint for the given clock signal.
    pub fn frames_due(&mut self, clock: Clock) -> u32 {
        match clock {
            Clock::Audio { queued_frames } => self.frames_for_audio(queued_frames),
            Clock::Wall { elapsed } => self.frames_for_wall_clock(elapsed),
        }
    }

    /// Drop any carried wall-clock remainder — call when emulation is paused so
    /// it doesn't lurch on resume.
    pub fn reset(&mut self) {
        self.frame_accum = 0.0;
    }

    /// Stereo sample-frames one emulated frame produces at this sample rate.
    fn samples_per_frame(&self) -> f64 {
        self.sample_rate as f64 / GB_FPS
    }

    fn frames_for_audio(&self, queued_frames: usize) -> u32 {
        let target = self.sample_rate as f64 * AUDIO_TARGET_SECS;
        let deficit = target - queued_frames as f64;
        if deficit <= 0.0 {
            return 0;
        }
        (deficit / self.samples_per_frame()).ceil() as u32
    }

    fn frames_for_wall_clock(&mut self, elapsed: Duration) -> u32 {
        let dt = elapsed.as_secs_f64().min(MAX_ELAPSED_SECS);
        self.frame_accum += dt * GB_FPS;
        let frames = (self.frame_accum as u32).min(WALL_MAX_FRAMES);
        self.frame_accum -= frames as f64;
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    #[test]
    fn full_queue_runs_nothing() {
        let mut p = Pacer::new(SR);
        // 60 ms target = 2880 stereo frames at 48 kHz; anything at/over that → 0.
        assert_eq!(p.frames_due(Clock::Audio { queued_frames: 2880 }), 0);
        assert_eq!(p.frames_due(Clock::Audio { queued_frames: 5000 }), 0);
    }

    #[test]
    fn dry_queue_tops_up_toward_target() {
        let mut p = Pacer::new(SR);
        // Empty queue: ~60 ms / ~13.4 ms per frame ≈ 4 frames.
        assert_eq!(p.frames_due(Clock::Audio { queued_frames: 0 }), 4);
    }

    #[test]
    fn partial_queue_runs_the_deficit() {
        let mut p = Pacer::new(SR);
        // 2000 queued, target 2880 → deficit 880 → ceil(880 / 803.6) = 2.
        assert_eq!(p.frames_due(Clock::Audio { queued_frames: 2000 }), 2);
    }

    #[test]
    fn audio_top_up_is_rate_independent() {
        // target and samples-per-frame both scale with the sample rate, so a
        // dry queue always tops up by the same small number of frames.
        for sr in [22_050, 44_100, 48_000, 96_000, 1_000_000] {
            let mut p = Pacer::new(sr);
            assert_eq!(p.frames_due(Clock::Audio { queued_frames: 0 }), 4, "sr={sr}");
        }
    }

    #[test]
    fn wall_clock_runs_a_frame_once_enough_time_passes() {
        let mut p = Pacer::new(SR);
        // 17 ms > one frame (16.74 ms) → 1 frame.
        assert_eq!(p.frames_due(Clock::Wall { elapsed: Duration::from_millis(17) }), 1);
    }

    #[test]
    fn wall_clock_accumulates_fractional_time() {
        let mut p = Pacer::new(SR);
        // 10 ms ≈ 0.597 frame: none yet, then the carry crosses a whole frame.
        assert_eq!(p.frames_due(Clock::Wall { elapsed: Duration::from_millis(10) }), 0);
        assert_eq!(p.frames_due(Clock::Wall { elapsed: Duration::from_millis(10) }), 1);
    }

    #[test]
    fn wall_clock_clamps_a_stall() {
        let mut p = Pacer::new(SR);
        // A 5-second stall must not fast-forward: clamped to 0.1 s → capped at 4.
        assert_eq!(
            p.frames_due(Clock::Wall { elapsed: Duration::from_secs(5) }),
            WALL_MAX_FRAMES
        );
    }

    #[test]
    fn reset_drops_carried_remainder() {
        let mut p = Pacer::new(SR);
        p.frames_due(Clock::Wall { elapsed: Duration::from_millis(10) }); // carry ~0.597
        p.reset();
        // Without the reset this second 10 ms would cross a frame; with it, none.
        assert_eq!(p.frames_due(Clock::Wall { elapsed: Duration::from_millis(10) }), 0);
    }
}
