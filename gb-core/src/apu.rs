//! Audio processing unit: 2 pulse channels, wave channel, noise channel.
//! Generates interleaved stereo f32 samples into an internal buffer that the
//! frontend drains each frame.

const WAVE_DUTY: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 0],
];

const CPU_HZ: u32 = 4_194_304;

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
struct Envelope {
    initial_volume: u8,
    direction_up: bool,
    period: u8,
    volume: u8,
    timer: u8,
}

impl Envelope {
    fn write(&mut self, val: u8) {
        self.initial_volume = val >> 4;
        self.direction_up = val & 0x08 != 0;
        self.period = val & 0x07;
    }
    fn read(&self) -> u8 {
        (self.initial_volume << 4) | (if self.direction_up { 8 } else { 0 }) | self.period
    }
    fn trigger(&mut self) {
        self.volume = self.initial_volume;
        self.timer = self.period;
    }
    fn clock(&mut self) {
        if self.period == 0 {
            return;
        }
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = self.period;
            if self.direction_up && self.volume < 15 {
                self.volume += 1;
            } else if !self.direction_up && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }
    /// DAC is on if the upper 5 bits of NRx2 are non-zero.
    fn dac_on(&self) -> bool {
        self.initial_volume != 0 || self.direction_up
    }
}

/// The length counter shared by all four channels: when enabled it counts
/// down on each frame-sequencer length clock and disables its channel at zero.
/// `max` is 64 for the pulse and noise channels, 256 for the wave channel.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct LengthCounter {
    counter: u16,
    enabled: bool,
    max: u16,
}

impl LengthCounter {
    fn new(max: u16) -> Self {
        LengthCounter { counter: 0, enabled: false, max }
    }

    /// NRx1 write: remaining length = max - written value.
    fn load(&mut self, written: u16) {
        self.counter = self.max - written;
    }

    /// NRx4 bit 6.
    fn set_enable(&mut self, enable: bool) {
        self.enabled = enable;
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// On trigger a zero counter reloads to its maximum.
    fn trigger(&mut self) {
        if self.counter == 0 {
            self.counter = self.max;
        }
    }

    /// Advance one length step; returns true when the channel should be
    /// disabled (the counter reached zero this step).
    fn clock(&mut self) -> bool {
        if self.enabled && self.counter > 0 {
            self.counter -= 1;
            if self.counter == 0 {
                return true;
            }
        }
        false
    }
}

/// The frequency timer shared by all channels: a down-counter reloaded to a
/// channel-supplied period. `step` advances it by `cycles` T-cycles and reports
/// how many times it expired, so each channel can advance its own waveform
/// (duty step, wave position, LFSR shift) that many times.
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
struct FrequencyTimer {
    counter: u32,
}

impl FrequencyTimer {
    /// Set the counter to `period` (on channel trigger).
    fn reload_to(&mut self, period: u32) {
        self.counter = period;
    }

    fn step(&mut self, cycles: u32, period: u32) -> u32 {
        let mut expirations = 0;
        let mut c = cycles;
        while c > 0 {
            let step = c.min(self.counter.max(1));
            if self.counter <= step {
                self.counter = period;
                expirations += 1;
                c -= step;
            } else {
                self.counter -= step;
                c = 0;
            }
        }
        expirations
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Pulse {
    enabled: bool,
    duty: u8,
    duty_pos: usize,
    freq: u16, // 11-bit
    timer: FrequencyTimer,
    length: LengthCounter,
    envelope: Envelope,
    // sweep (channel 1 only)
    sweep_period: u8,
    sweep_up: bool,
    sweep_shift: u8,
    sweep_timer: u8,
    sweep_shadow: u16,
    sweep_enabled: bool,
}

impl Default for Pulse {
    fn default() -> Self {
        Pulse {
            enabled: false,
            duty: 0,
            duty_pos: 0,
            freq: 0,
            timer: FrequencyTimer::default(),
            length: LengthCounter::new(64),
            envelope: Envelope::default(),
            sweep_period: 0,
            sweep_up: false,
            sweep_shift: 0,
            sweep_timer: 0,
            sweep_shadow: 0,
            sweep_enabled: false,
        }
    }
}

impl Pulse {
    /// Frequency-timer period in T-cycles for the current frequency.
    fn period(&self) -> u32 {
        (2048 - self.freq as u32) * 4
    }

    fn trigger(&mut self, has_sweep: bool) {
        self.enabled = self.envelope.dac_on();
        self.length.trigger();
        self.timer.reload_to(self.period());
        self.envelope.trigger();
        if has_sweep {
            self.sweep_shadow = self.freq;
            self.sweep_timer = if self.sweep_period == 0 { 8 } else { self.sweep_period };
            self.sweep_enabled = self.sweep_period != 0 || self.sweep_shift != 0;
            if self.sweep_shift != 0 && self.sweep_next() > 2047 {
                self.enabled = false;
            }
        }
    }

    fn sweep_next(&self) -> u32 {
        let delta = self.sweep_shadow >> self.sweep_shift;
        if self.sweep_up {
            self.sweep_shadow as u32 + delta as u32
        } else {
            (self.sweep_shadow.wrapping_sub(delta)) as u32
        }
    }

    fn clock_sweep(&mut self) {
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }
        if self.sweep_timer == 0 {
            self.sweep_timer = if self.sweep_period == 0 { 8 } else { self.sweep_period };
            if self.sweep_enabled && self.sweep_period != 0 {
                let next = self.sweep_next();
                if next <= 2047 && self.sweep_shift != 0 {
                    self.sweep_shadow = next as u16;
                    self.freq = next as u16;
                    if self.sweep_next() > 2047 {
                        self.enabled = false;
                    }
                } else if next > 2047 {
                    self.enabled = false;
                }
            }
        }
    }

    fn clock_length(&mut self) {
        if self.length.clock() {
            self.enabled = false;
        }
    }

    fn tick(&mut self, cycles: u32) {
        let expirations = self.timer.step(cycles, self.period());
        for _ in 0..expirations {
            self.duty_pos = (self.duty_pos + 1) % 8;
        }
    }

    fn output(&self) -> f32 {
        if !self.enabled || !self.envelope.dac_on() {
            return 0.0;
        }
        let bit = WAVE_DUTY[self.duty as usize][self.duty_pos];
        let dac_in = bit as f32 * self.envelope.volume as f32 / 15.0;
        dac_in * 2.0 - 1.0
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Wave {
    enabled: bool,
    dac_on: bool,
    volume_shift: u8, // 0=mute,1=100%,2=50%,3=25% encoded as shifts 4,0,1,2
    freq: u16,
    timer: FrequencyTimer,
    length: LengthCounter,
    pos: usize,
    pub table: [u8; 16],
    sample: u8,
}

impl Default for Wave {
    fn default() -> Self {
        Wave {
            enabled: false,
            dac_on: false,
            volume_shift: 0,
            freq: 0,
            timer: FrequencyTimer::default(),
            length: LengthCounter::new(256),
            pos: 0,
            table: [0; 16],
            sample: 0,
        }
    }
}

impl Wave {
    /// Frequency-timer period in T-cycles for the current frequency.
    fn period(&self) -> u32 {
        (2048 - self.freq as u32) * 2
    }

    fn trigger(&mut self) {
        self.enabled = self.dac_on;
        self.length.trigger();
        self.timer.reload_to(self.period());
        self.pos = 0;
    }

    fn clock_length(&mut self) {
        if self.length.clock() {
            self.enabled = false;
        }
    }

    fn tick(&mut self, cycles: u32) {
        let expirations = self.timer.step(cycles, self.period());
        for _ in 0..expirations {
            self.pos = (self.pos + 1) % 32;
            let byte = self.table[self.pos / 2];
            self.sample = if self.pos.is_multiple_of(2) { byte >> 4 } else { byte & 0x0F };
        }
    }

    fn output(&self) -> f32 {
        if !self.enabled || !self.dac_on {
            return 0.0;
        }
        let shift = match self.volume_shift {
            0 => 4,
            1 => 0,
            2 => 1,
            _ => 2,
        };
        let dac_in = (self.sample >> shift) as f32 / 15.0;
        dac_in * 2.0 - 1.0
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Noise {
    enabled: bool,
    length: LengthCounter,
    envelope: Envelope,
    shift: u8,
    width7: bool,
    divisor_code: u8,
    timer: FrequencyTimer,
    lfsr: u16,
}

impl Default for Noise {
    fn default() -> Self {
        Noise {
            enabled: false,
            length: LengthCounter::new(64),
            envelope: Envelope::default(),
            shift: 0,
            width7: false,
            divisor_code: 0,
            timer: FrequencyTimer::default(),
            lfsr: 0,
        }
    }
}

impl Noise {
    fn period(&self) -> u32 {
        let d = match self.divisor_code {
            0 => 8,
            n => (n as u32) * 16,
        };
        d << self.shift
    }

    fn trigger(&mut self) {
        self.enabled = self.envelope.dac_on();
        self.length.trigger();
        self.timer.reload_to(self.period());
        self.envelope.trigger();
        self.lfsr = 0x7FFF;
    }

    fn clock_length(&mut self) {
        if self.length.clock() {
            self.enabled = false;
        }
    }

    fn tick(&mut self, cycles: u32) {
        let expirations = self.timer.step(cycles, self.period());
        for _ in 0..expirations {
            let xor = (self.lfsr & 1) ^ ((self.lfsr >> 1) & 1);
            self.lfsr = (self.lfsr >> 1) | (xor << 14);
            if self.width7 {
                self.lfsr = (self.lfsr & !(1 << 6)) | (xor << 6);
            }
        }
    }

    fn output(&self) -> f32 {
        if !self.enabled || !self.envelope.dac_on() {
            return 0.0;
        }
        let bit = (!self.lfsr & 1) as f32;
        let dac_in = bit * self.envelope.volume as f32 / 15.0;
        dac_in * 2.0 - 1.0
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Apu {
    enabled: bool,
    ch1: Pulse,
    ch2: Pulse,
    ch3: Wave,
    ch4: Noise,
    nr50: u8,
    nr51: u8,
    frame_seq: u8,
    frame_timer: u32,
    sample_rate: u32,
    sample_counter: u32,
    /// Interleaved stereo samples produced since last drain. Transient output,
    /// not machine state — excluded from save states.
    #[serde(skip)]
    pub samples: Vec<f32>,
}

impl Default for Apu {
    fn default() -> Self {
        Apu {
            enabled: true,
            ch1: Pulse::default(),
            ch2: Pulse::default(),
            ch3: Wave::default(),
            ch4: Noise::default(),
            nr50: 0x77,
            nr51: 0xF3,
            frame_seq: 0,
            frame_timer: 0,
            sample_rate: 48_000,
            sample_counter: 0,
            samples: Vec::new(),
        }
    }
}

impl Apu {
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate.max(8000);
    }

    pub fn drain_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.samples)
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF10 => {
                0x80 | (self.ch1.sweep_period << 4)
                    | (if self.ch1.sweep_up { 0 } else { 8 })
                    | self.ch1.sweep_shift
            }
            0xFF11 => (self.ch1.duty << 6) | 0x3F,
            0xFF12 => self.ch1.envelope.read(),
            0xFF13 => 0xFF,
            0xFF14 => 0xBF | (if self.ch1.length.is_enabled() { 0x40 } else { 0 }),
            0xFF16 => (self.ch2.duty << 6) | 0x3F,
            0xFF17 => self.ch2.envelope.read(),
            0xFF18 => 0xFF,
            0xFF19 => 0xBF | (if self.ch2.length.is_enabled() { 0x40 } else { 0 }),
            0xFF1A => 0x7F | (if self.ch3.dac_on { 0x80 } else { 0 }),
            0xFF1B => 0xFF,
            0xFF1C => 0x9F | (self.ch3.volume_shift << 5),
            0xFF1D => 0xFF,
            0xFF1E => 0xBF | (if self.ch3.length.is_enabled() { 0x40 } else { 0 }),
            0xFF20 => 0xFF,
            0xFF21 => self.ch4.envelope.read(),
            0xFF22 => (self.ch4.shift << 4) | (if self.ch4.width7 { 8 } else { 0 }) | self.ch4.divisor_code,
            0xFF23 => 0xBF | (if self.ch4.length.is_enabled() { 0x40 } else { 0 }),
            0xFF24 => self.nr50,
            0xFF25 => self.nr51,
            0xFF26 => {
                let mut v = 0x70;
                if self.enabled {
                    v |= 0x80;
                }
                if self.ch1.enabled {
                    v |= 1;
                }
                if self.ch2.enabled {
                    v |= 2;
                }
                if self.ch3.enabled {
                    v |= 4;
                }
                if self.ch4.enabled {
                    v |= 8;
                }
                v
            }
            0xFF30..=0xFF3F => self.ch3.table[(addr - 0xFF30) as usize],
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        if !self.enabled && addr != 0xFF26 && !(0xFF30..=0xFF3F).contains(&addr) {
            return;
        }
        match addr {
            0xFF10 => {
                self.ch1.sweep_period = (val >> 4) & 7;
                self.ch1.sweep_up = val & 8 == 0;
                self.ch1.sweep_shift = val & 7;
            }
            0xFF11 => {
                self.ch1.duty = val >> 6;
                self.ch1.length.load((val & 0x3F) as u16);
            }
            0xFF12 => {
                self.ch1.envelope.write(val);
                if !self.ch1.envelope.dac_on() {
                    self.ch1.enabled = false;
                }
            }
            0xFF13 => self.ch1.freq = (self.ch1.freq & 0x700) | val as u16,
            0xFF14 => {
                self.ch1.freq = (self.ch1.freq & 0xFF) | ((val as u16 & 7) << 8);
                self.ch1.length.set_enable(val & 0x40 != 0);
                if val & 0x80 != 0 {
                    self.ch1.trigger(true);
                }
            }
            0xFF16 => {
                self.ch2.duty = val >> 6;
                self.ch2.length.load((val & 0x3F) as u16);
            }
            0xFF17 => {
                self.ch2.envelope.write(val);
                if !self.ch2.envelope.dac_on() {
                    self.ch2.enabled = false;
                }
            }
            0xFF18 => self.ch2.freq = (self.ch2.freq & 0x700) | val as u16,
            0xFF19 => {
                self.ch2.freq = (self.ch2.freq & 0xFF) | ((val as u16 & 7) << 8);
                self.ch2.length.set_enable(val & 0x40 != 0);
                if val & 0x80 != 0 {
                    self.ch2.trigger(false);
                }
            }
            0xFF1A => {
                self.ch3.dac_on = val & 0x80 != 0;
                if !self.ch3.dac_on {
                    self.ch3.enabled = false;
                }
            }
            0xFF1B => self.ch3.length.load(val as u16),
            0xFF1C => self.ch3.volume_shift = (val >> 5) & 3,
            0xFF1D => self.ch3.freq = (self.ch3.freq & 0x700) | val as u16,
            0xFF1E => {
                self.ch3.freq = (self.ch3.freq & 0xFF) | ((val as u16 & 7) << 8);
                self.ch3.length.set_enable(val & 0x40 != 0);
                if val & 0x80 != 0 {
                    self.ch3.trigger();
                }
            }
            0xFF20 => self.ch4.length.load((val & 0x3F) as u16),
            0xFF21 => {
                self.ch4.envelope.write(val);
                if !self.ch4.envelope.dac_on() {
                    self.ch4.enabled = false;
                }
            }
            0xFF22 => {
                self.ch4.shift = val >> 4;
                self.ch4.width7 = val & 8 != 0;
                self.ch4.divisor_code = val & 7;
            }
            0xFF23 => {
                self.ch4.length.set_enable(val & 0x40 != 0);
                if val & 0x80 != 0 {
                    self.ch4.trigger();
                }
            }
            0xFF24 => self.nr50 = val,
            0xFF25 => self.nr51 = val,
            0xFF26 => {
                let on = val & 0x80 != 0;
                if !on && self.enabled {
                    // Power off clears everything.
                    let table = self.ch3.table;
                    let rate = self.sample_rate;
                    let samples = std::mem::take(&mut self.samples);
                    *self = Apu::default();
                    self.enabled = false;
                    self.nr50 = 0;
                    self.nr51 = 0;
                    self.ch3.table = table;
                    self.sample_rate = rate;
                    self.samples = samples;
                }
                self.enabled = on;
            }
            0xFF30..=0xFF3F => self.ch3.table[(addr - 0xFF30) as usize] = val,
            _ => {}
        }
    }

    fn clock_frame_sequencer(&mut self) {
        // 512 Hz sequencer: lengths on even steps, sweep on 2/6, envelope on 7.
        if self.frame_seq.is_multiple_of(2) {
            self.ch1.clock_length();
            self.ch2.clock_length();
            self.ch3.clock_length();
            self.ch4.clock_length();
        }
        if self.frame_seq == 2 || self.frame_seq == 6 {
            self.ch1.clock_sweep();
        }
        if self.frame_seq == 7 {
            self.ch1.envelope.clock();
            self.ch2.envelope.clock();
            self.ch4.envelope.clock();
        }
        self.frame_seq = (self.frame_seq + 1) % 8;
    }

    pub fn tick(&mut self, cycles: u32) {
        if !self.enabled {
            // Still produce silence so audio timing stays consistent.
            self.sample_counter += cycles * self.sample_rate;
            while self.sample_counter >= CPU_HZ {
                self.sample_counter -= CPU_HZ;
                self.samples.push(0.0);
                self.samples.push(0.0);
            }
            return;
        }
        for _ in 0..cycles {
            self.frame_timer += 1;
            if self.frame_timer >= 8192 {
                self.frame_timer = 0;
                self.clock_frame_sequencer();
            }
            self.ch1.tick(1);
            self.ch2.tick(1);
            self.ch3.tick(1);
            self.ch4.tick(1);
            self.sample_counter += self.sample_rate;
            if self.sample_counter >= CPU_HZ {
                self.sample_counter -= CPU_HZ;
                self.push_sample();
            }
        }
    }

    fn push_sample(&mut self) {
        let outs = [
            self.ch1.output(),
            self.ch2.output(),
            self.ch3.output(),
            self.ch4.output(),
        ];
        let mut left = 0.0;
        let mut right = 0.0;
        for (i, o) in outs.iter().enumerate() {
            if self.nr51 & (1 << (4 + i)) != 0 {
                left += o;
            }
            if self.nr51 & (1 << i) != 0 {
                right += o;
            }
        }
        let lvol = ((self.nr50 >> 4) & 7) as f32 + 1.0;
        let rvol = (self.nr50 & 7) as f32 + 1.0;
        self.samples.push(left / 4.0 * lvol / 8.0 * 0.5);
        self.samples.push(right / 4.0 * rvol / 8.0 * 0.5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_counter_disables_at_zero() {
        let mut lc = LengthCounter::new(64);
        lc.load(63); // remaining = 1
        lc.set_enable(true);
        assert!(lc.clock(), "counter should hit zero and signal disable");
        assert!(!lc.clock(), "no further disable once at zero");
    }

    #[test]
    fn length_counter_only_clocks_when_enabled() {
        let mut lc = LengthCounter::new(64);
        lc.load(63);
        // Not enabled: clocking is a no-op.
        assert!(!lc.clock());
        lc.set_enable(true);
        assert!(lc.clock());
    }

    #[test]
    fn length_counter_trigger_reloads_only_when_zero() {
        let mut lc = LengthCounter::new(64);
        lc.trigger(); // was zero -> reloads to max
        lc.set_enable(true);
        // 64 clocks to reach zero.
        for _ in 0..63 {
            assert!(!lc.clock());
        }
        assert!(lc.clock());

        let mut lc = LengthCounter::new(256);
        lc.load(200); // remaining 56
        lc.trigger(); // non-zero -> unchanged
        lc.set_enable(true);
        for _ in 0..55 {
            assert!(!lc.clock());
        }
        assert!(lc.clock());
    }

    #[test]
    fn frequency_timer_counts_expirations() {
        let mut t = FrequencyTimer::default();
        t.reload_to(4);
        // 4 cycles per expiry: 16 cycles -> 4 expirations.
        assert_eq!(t.step(16, 4), 4);
        // Partial: 2 more cycles, none yet; 2 more completes one.
        assert_eq!(t.step(2, 4), 0);
        assert_eq!(t.step(2, 4), 1);
    }

    #[test]
    fn frequency_timer_uses_period_at_reload() {
        let mut t = FrequencyTimer::default();
        t.reload_to(8);
        // First expiry after 8 cycles, then reloads to the period passed in.
        assert_eq!(t.step(8, 8), 1);
        assert_eq!(t.step(8, 8), 1);
    }

    #[test]
    fn produces_samples_at_requested_rate() {
        let mut apu = Apu::default();
        apu.set_sample_rate(48_000);
        apu.tick(CPU_HZ / 10); // 100 ms
        let n = apu.samples.len() / 2;
        assert!((4700..=4900).contains(&n), "got {n} samples");
    }

    #[test]
    fn triggered_pulse_channel_reports_on() {
        let mut apu = Apu::default();
        apu.write(0xFF12, 0xF0); // full volume envelope
        apu.write(0xFF13, 0x00);
        apu.write(0xFF14, 0x87); // trigger, freq high bits
        assert!(apu.read(0xFF26) & 0x01 != 0);
    }

    #[test]
    fn length_counter_disables_channel() {
        let mut apu = Apu::default();
        apu.write(0xFF12, 0xF0);
        apu.write(0xFF11, 0x3F); // length load = 63 -> counter = 1
        apu.write(0xFF14, 0xC0 | 0x80); // trigger with length enable
        // One frame-sequencer length clock happens within 16384 cycles.
        apu.tick(16384);
        assert!(apu.read(0xFF26) & 0x01 == 0);
    }

    #[test]
    fn apu_power_off_clears_registers() {
        let mut apu = Apu::default();
        apu.write(0xFF25, 0xAB);
        apu.write(0xFF26, 0x00);
        assert_eq!(apu.read(0xFF25), 0x00);
        assert_eq!(apu.read(0xFF26) & 0x80, 0);
        // Writes ignored while off.
        apu.write(0xFF25, 0x55);
        assert_eq!(apu.read(0xFF25), 0x00);
    }

    #[test]
    fn wave_channel_produces_output_when_triggered() {
        // Regression: Tetris's in-stage bassline lives on the wave channel,
        // which is idle on the title screen. Make sure a triggered wave
        // channel actually contributes non-zero samples.
        let mut apu = Apu::default();
        apu.set_sample_rate(48_000);
        // Fill wave RAM with a ramp and enable the channel at full volume.
        for i in 0..16 {
            apu.write(0xFF30 + i, 0x0F);
        }
        apu.write(0xFF1A, 0x80); // DAC on
        apu.write(0xFF1C, 0x20); // volume 100%
        apu.write(0xFF1D, 0x00); // freq low
        apu.write(0xFF1E, 0x87); // trigger, freq high
        assert!(apu.read(0xFF26) & 0x04 != 0, "wave channel should be on");
        apu.tick(48_000); // 1 s
        let energy: f32 = apu.samples.iter().map(|s| s * s).sum();
        assert!(energy > 0.0, "wave channel produced silence");
    }

    #[test]
    fn pulse_dac_off_silences_channel() {
        let mut apu = Apu::default();
        apu.write(0xFF12, 0xF0);
        apu.write(0xFF14, 0x80);
        assert!(apu.read(0xFF26) & 1 != 0);
        apu.write(0xFF12, 0x00); // DAC off
        assert!(apu.read(0xFF26) & 1 == 0);
    }
}
