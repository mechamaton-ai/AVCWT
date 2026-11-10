use std::fs;
use std::path::Path;

/// Persistent application settings stored in `settings.ini`, compatible with the
/// original `QSettings("settings.ini", QSettings.Format.IniFormat)` configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    pub host_address: String,
    pub cache_size: i64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            host_address: "localhost:8765".to_string(),
            cache_size: 150,
        }
    }
}

impl AppSettings {
    pub const DEFAULT_PATH: &'static str = "settings.ini";

    /// Loads settings from an INI file if it exists, falling back to defaults.
    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Self {
        let mut settings = Self::default();
        if let Ok(contents) = fs::read_to_string(path) {
            for raw_line in contents.lines() {
                let line = raw_line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[') {
                    continue;
                }
                if let Some((key, val)) = line.split_once('=') {
                    let key = key.trim();
                    let val = val.trim().trim_matches('"');
                    match key {
                        "host_address" if !val.is_empty() => {
                            settings.host_address = val.to_string();
                        }
                        "cache_size" => {
                            if let Ok(parsed) = val.parse::<i64>() {
                                settings.cache_size = parsed.clamp(10, 2000);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        settings
    }

    /// Saves settings to an INI file in `QSettings` `[General]` format.
    pub fn save_to_path<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let content = format!(
            "[General]\nhost_address={}\ncache_size={}\n",
            self.host_address, self.cache_size
        );
        fs::write(path, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_roundtrip() {
        let temp_path = std::env::temp_dir().join("avcwt_test_settings.ini");
        let _ = fs::remove_file(&temp_path);

        let def = AppSettings::load_from_path(&temp_path);
        assert_eq!(def.host_address, "localhost:8765");
        assert_eq!(def.cache_size, 150);

        let custom = AppSettings {
            host_address: "10.0.0.5:8765".to_string(),
            cache_size: 500,
        };
        custom.save_to_path(&temp_path).unwrap();

        let loaded = AppSettings::load_from_path(&temp_path);
        assert_eq!(loaded, custom);

        let _ = fs::remove_file(&temp_path);
    }
}
