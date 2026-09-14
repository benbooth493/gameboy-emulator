//! Audio output: cpal stream fed from a shared sample queue.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct Audio {
    _stream: Option<cpal::Stream>,
    queue: Arc<Mutex<VecDeque<f32>>>,
    pub sample_rate: u32,
    /// True when a real output stream was opened; used to decide whether the
    /// audio device can act as the emulation clock.
    pub available: bool,
}

impl Audio {
    /// Try to open the default output device; on failure the emulator still
    /// runs, just silently.
    pub fn new() -> Self {
        let queue: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let mut sample_rate = 48_000;
        let stream = (|| {
            let host = cpal::default_host();
            let device = host.default_output_device()?;
            let config = device.default_output_config().ok()?;
            sample_rate = config.sample_rate().0;
            let channels = config.channels() as usize;
            let q = queue.clone();
            let debug = std::env::var_os("GBEM_AUDIO_DEBUG").is_some();
            let mut filled = 0u64;
            let mut starved = 0u64;
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            let mut last_report = std::time::Instant::now();
            let stream = device
                .build_output_stream(
                    &config.into(),
                    move |out: &mut [f32], _| {
                        let mut q = q.lock().unwrap();
                        for frame in out.chunks_mut(channels) {
                            let l = q.pop_front();
                            let r = q.pop_front();
                            if l.is_none() {
                                starved += 1;
                            } else {
                                filled += 1;
                            }
                            let l = l.unwrap_or(0.0);
                            let r = r.unwrap_or(l);
                            lo = lo.min(l);
                            hi = hi.max(l);
                            for (i, s) in frame.iter_mut().enumerate() {
                                *s = if i % 2 == 0 { l } else { r };
                            }
                        }
                        if debug && last_report.elapsed().as_secs() >= 1 {
                            eprintln!(
                                "[audio] filled={filled} starved={starved} queued={} p2p={:.3}",
                                q.len() / 2,
                                if hi >= lo { hi - lo } else { 0.0 }
                            );
                            filled = 0;
                            starved = 0;
                            lo = f32::MAX;
                            hi = f32::MIN;
                            last_report = std::time::Instant::now();
                        }
                    },
                    |e| eprintln!("audio error: {e}"),
                    None,
                )
                .ok()?;
            stream.play().ok()?;
            Some(stream)
        })();
        Audio {
            available: stream.is_some(),
            _stream: stream,
            queue,
            sample_rate,
        }
    }

    pub fn push_samples(&self, samples: &[f32]) {
        let mut q = self.queue.lock().unwrap();
        // Hard ceiling to bound latency if the producer ever races ahead
        // (e.g. window unfocused). Drop the OLDEST audio, not the newest, so a
        // burst never leaves a gap in the middle of the current sound.
        let cap = (self.sample_rate as usize * 2) * 250 / 1000;
        q.extend(samples.iter().copied());
        while q.len() > cap {
            q.pop_front();
        }
    }

    /// Queued stereo sample-frames — used to pace emulation against audio.
    pub fn queued_frames(&self) -> usize {
        self.queue.lock().unwrap().len() / 2
    }
}
