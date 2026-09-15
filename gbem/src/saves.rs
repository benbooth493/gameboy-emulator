//! Battery-backed save files: the cartridge RAM persisted next to the ROM as
//! `<rom>.sav`.

use std::path::{Path, PathBuf};

/// The save-file path for a ROM: same location, `.sav` extension.
pub fn save_path_for(rom: &Path) -> PathBuf {
    rom.with_extension("sav")
}

/// The save-state path for a ROM: same location, `.state` extension.
pub fn state_path_for(rom: &Path) -> PathBuf {
    rom.with_extension("state")
}

/// Read a save file if it exists (returns `None` on any error).
pub fn load(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// Write cartridge RAM to the save file.
pub fn write(path: &Path, ram: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, ram)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_path_swaps_extension() {
        assert_eq!(save_path_for(Path::new("/games/tetris.gb")), PathBuf::from("/games/tetris.sav"));
        assert_eq!(save_path_for(Path::new("zelda.gbc")), PathBuf::from("zelda.sav"));
    }

    #[test]
    fn save_path_adds_extension_when_missing() {
        assert_eq!(save_path_for(Path::new("rom")), PathBuf::from("rom.sav"));
    }

    #[test]
    fn state_path_swaps_extension() {
        assert_eq!(state_path_for(Path::new("/g/tetris.gb")), PathBuf::from("/g/tetris.state"));
    }
}
