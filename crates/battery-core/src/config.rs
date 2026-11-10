use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// UI language. English by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En,
    Es,
}

impl Lang {
    /// Pick the string for this language.
    pub fn t(self, en: &'static str, es: &'static str) -> &'static str {
        match self {
            Lang::En => en,
            Lang::Es => es,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub lang: Lang,
    pub poll_secs: u64,
    pub low_threshold: u8,
    pub notify: bool,
    pub autostart: bool,
    pub dark: bool,
    pub widget: WidgetConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetConfig {
    pub visible: bool,
    pub always_on_top: bool,
    pub position: Option<(i32, i32)>,
    /// 0.3..=1.0, applied to the widget background.
    pub opacity: f32,
    /// Device ids hidden from the widget; new devices show by default.
    pub hidden: BTreeSet<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self { lang: Lang::En, poll_secs: 60, low_threshold: 20, notify: true, autostart: false, dark: true, widget: WidgetConfig::default() }
    }
}

impl Default for WidgetConfig {
    fn default() -> Self {
        Self { visible: true, always_on_top: true, position: None, opacity: 0.9, hidden: BTreeSet::new() }
    }
}

impl Config {
    pub const POLL_RANGE: (u64, u64) = (15, 600);

    /// Packaged (MSIX): the package's real LocalState folder, which other
    /// programs (Notepad for the log) can open; %APPDATA% would be redirected
    /// into a private per-package copy. Unpackaged: %APPDATA%\LowBattery.
    pub fn path() -> PathBuf {
        if let (Some(pfn), Some(local)) = (package_family_name(), std::env::var_os("LOCALAPPDATA")) {
            return PathBuf::from(local).join("Packages").join(pfn).join("LocalState").join("config.json");
        }
        let base = std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("LowBattery").join("config.json")
    }

    /// Missing or corrupt file -> defaults (the corrupt file is kept as .bak).
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Config>(&bytes).map(Config::sanitized).unwrap_or_else(|e| {
                log::warn!("config unreadable ({e}), using defaults");
                let _ = std::fs::rename(&path, path.with_extension("json.bak"));
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    /// Write to a temp file then rename, so a crash never leaves half a file.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn sanitized(mut self) -> Self {
        self.poll_secs = self.poll_secs.clamp(Self::POLL_RANGE.0, Self::POLL_RANGE.1);
        self.low_threshold = self.low_threshold.clamp(5, 50);
        self.widget.opacity = self.widget.opacity.clamp(0.3, 1.0);
        self
    }
}

/// Package family name when running from an MSIX package, else None.
#[cfg(windows)]
pub fn package_family_name() -> Option<String> {
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFamilyName;
    let mut len = 0u32;
    // First call sizes the buffer; APPMODEL_ERROR_NO_PACKAGE (15700) when unpackaged.
    unsafe { GetCurrentPackageFamilyName(&mut len, std::ptr::null_mut()) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    if unsafe { GetCurrentPackageFamilyName(&mut len, buf.as_mut_ptr()) } != 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len.saturating_sub(1) as usize]))
}

#[cfg(not(windows))]
pub fn package_family_name() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_json_fills_defaults_and_clamps() {
        let c: Config = serde_json::from_str(r#"{"poll_secs": 1, "widget": {"opacity": 5.0}}"#).unwrap();
        let c = c.sanitized();
        assert_eq!(c.poll_secs, 15);
        assert_eq!(c.widget.opacity, 1.0);
        assert!(c.widget.always_on_top);
        assert_eq!(c.low_threshold, 20);
        assert_eq!(c.lang, Lang::En); // configs from before the setting default to English
        let es: Config = serde_json::from_str(r#"{"lang": "es"}"#).unwrap();
        assert_eq!(es.lang.t("Devices", "Dispositivos"), "Dispositivos");
    }
}
