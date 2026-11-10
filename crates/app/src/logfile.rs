//! Plain-text log next to config.json, readable in Notepad. Backs the `log`
//! crate, so every `log::info!/warn!/error!` in core and app lands here.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 1024 * 1024;

pub fn path() -> PathBuf {
    battery_core::config::Config::path().with_file_name("lowbattery.log")
}

struct FileLog(Mutex<File>);

impl log::Log for FileLog {
    /// Normal run: ours at Info, third-party crates only warnings. `--debug`:
    /// everything at Debug, including creamui's resize / scale-factor events.
    fn enabled(&self, m: &log::Metadata) -> bool {
        let ours = m.target().starts_with("lowbattery") || m.target().starts_with("battery_core");
        let debug = log::max_level() >= log::LevelFilter::Debug;
        m.level() <= if ours || debug { log::max_level() } else { log::LevelFilter::Warn }
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        if let Ok(mut f) = self.0.lock() {
            let _ = writeln!(f, "{} {:<5} [{}] {}", timestamp(), r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {
        if let Ok(mut f) = self.0.lock() {
            let _ = f.flush();
        }
    }
}

/// Opens the log (one-generation rotation at 1 MiB) and logs panics too.
/// Never fails the app: without a writable log it just runs silently.
pub fn init(debug: bool) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&p, p.with_extension("old.log"));
    }
    let Ok(file) = OpenOptions::new().create(true).append(true).open(&p) else { return };
    // Leaked on purpose: the logger lives for the whole process.
    if log::set_logger(Box::leak(Box::new(FileLog(Mutex::new(file))))).is_err() {
        return;
    }
    log::set_max_level(if debug { log::LevelFilter::Debug } else { log::LevelFilter::Info });
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("PANIC: {info}");
        log::logger().flush();
        default_hook(info);
    }));
}

/// UTC "YYYY-MM-DD HH:MM:SS" without a date crate (civil-from-days).
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    format_utc(secs)
}

fn format_utc(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_dates() {
        assert_eq!(super::format_utc(0), "1970-01-01 00:00:00Z");
        assert_eq!(super::format_utc(1_709_210_096), "2024-02-29 12:34:56Z");
    }
}
