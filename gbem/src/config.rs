//! Persisted, remappable input bindings: one keyboard key and one gamepad
//! button per Game Boy button. Stored as human-readable JSON in the OS config
//! directory so it survives across runs.

use eframe::egui::Key;
use gb_core::Button as Gb;
use gilrs::Button as Pad;

/// The eight Game Boy buttons, in a fixed order used to index the binding
/// arrays and to render the settings table.
pub const BUTTONS: [(&str, Gb); 8] = [
    ("Up", Gb::Up),
    ("Down", Gb::Down),
    ("Left", Gb::Left),
    ("Right", Gb::Right),
    ("A", Gb::A),
    ("B", Gb::B),
    ("Start", Gb::Start),
    ("Select", Gb::Select),
];

#[derive(Clone)]
pub struct Controls {
    pub keys: [Key; 8],
    pub pads: [Pad; 8],
}

impl Default for Controls {
    fn default() -> Self {
        Controls {
            keys: [
                Key::ArrowUp,
                Key::ArrowDown,
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::Z,
                Key::X,
                Key::Enter,
                Key::Backspace,
            ],
            pads: [
                Pad::DPadUp,
                Pad::DPadDown,
                Pad::DPadLeft,
                Pad::DPadRight,
                Pad::South,
                Pad::East,
                Pad::Start,
                Pad::Select,
            ],
        }
    }
}

impl Controls {
    /// Load bindings from disk, falling back to the defaults for anything
    /// missing or unparseable.
    pub fn load() -> Self {
        let mut c = Controls::default();
        let Some(text) = config_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return c;
        };
        let Ok(dto) = serde_json::from_str::<Dto>(&text) else {
            return c;
        };
        for i in 0..8 {
            if let Some(k) = dto.keys.get(i).and_then(|s| key_from_name(s)) {
                c.keys[i] = k;
            }
            if let Some(p) = dto.pads.get(i).and_then(|s| pad_from_name(s)) {
                c.pads[i] = p;
            }
        }
        c
    }

    pub fn save(&self) {
        let dto = Dto {
            keys: self.keys.iter().map(|k| k.name().to_string()).collect(),
            pads: self.pads.iter().map(|p| pad_name(*p).to_string()).collect(),
        };
        let Some(path) = config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&dto) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Dto {
    keys: Vec<String>,
    pads: Vec<String>,
}

fn config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("gbem").join("controls.json"))
}

fn key_from_name(name: &str) -> Option<Key> {
    Key::ALL.iter().copied().find(|k| k.name() == name)
}

/// A short display/serialization name for a gamepad button.
pub fn pad_name(b: Pad) -> &'static str {
    use Pad::*;
    match b {
        South => "South",
        East => "East",
        North => "North",
        West => "West",
        C => "C",
        Z => "Z",
        LeftTrigger => "L1",
        LeftTrigger2 => "L2",
        RightTrigger => "R1",
        RightTrigger2 => "R2",
        Select => "Select",
        Start => "Start",
        Mode => "Mode",
        LeftThumb => "LStick",
        RightThumb => "RStick",
        DPadUp => "DPadUp",
        DPadDown => "DPadDown",
        DPadLeft => "DPadLeft",
        DPadRight => "DPadRight",
        Unknown => "Unknown",
    }
}

fn pad_from_name(name: &str) -> Option<Pad> {
    use Pad::*;
    Some(match name {
        "South" => South,
        "East" => East,
        "North" => North,
        "West" => West,
        "C" => C,
        "Z" => Z,
        "L1" => LeftTrigger,
        "L2" => LeftTrigger2,
        "R1" => RightTrigger,
        "R2" => RightTrigger2,
        "Select" => Select,
        "Start" => Start,
        "Mode" => Mode,
        "LStick" => LeftThumb,
        "RStick" => RightThumb,
        "DPadUp" => DPadUp,
        "DPadDown" => DPadDown,
        "DPadLeft" => DPadLeft,
        "DPadRight" => DPadRight,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_name_roundtrips() {
        for b in [Pad::South, Pad::East, Pad::DPadUp, Pad::Start, Pad::LeftTrigger] {
            assert_eq!(pad_from_name(pad_name(b)), Some(b));
        }
    }

    #[test]
    fn key_name_roundtrips() {
        for k in [Key::ArrowUp, Key::Z, Key::Enter, Key::Backspace] {
            assert_eq!(key_from_name(k.name()), Some(k));
        }
    }
}
