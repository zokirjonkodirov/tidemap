//! The user-editable `style.json`: loading, hot reload and saving.
//!
//! The file holds a pretty-printed `StyleSheet`. Missing fields take their defaults and absurd
//! values are clamped. A file that does not parse never replaces the last good sheet; the
//! problem is kept in `error` for the UI.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use mermaidlite_diagram::StyleSheet;

const FILE_NAME: &str = "style.json";
/// The file is looked at most this often.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Modification time and length: a cheap fingerprint of the file on disk.
type Stamp = (Option<SystemTime>, u64);

pub struct StyleFile {
    pub path: PathBuf,
    /// The last sheet that parsed (or the defaults).
    pub sheet: StyleSheet,
    /// Why the file on disk is not in use, as `style.json line N: message`.
    pub error: Option<String>,
    stamp: Option<Stamp>,
    last_check: Option<Instant>,
}

impl StyleFile {
    /// Reads `<dir>/style.json`, creating it with the defaults when it does not exist.
    pub fn load(dir: &Path) -> StyleFile {
        let mut file = StyleFile {
            path: dir.join(FILE_NAME),
            sheet: StyleSheet::default(),
            error: None,
            stamp: None,
            last_check: None,
        };
        if file.path.exists() {
            file.reload();
        } else if let Err(reason) = file.write(&StyleSheet::default()) {
            file.error = Some(reason);
        }
        file
    }

    /// Looks for an edit, at most once per second. True when `sheet` changed.
    pub fn poll(&mut self, now: Instant) -> bool {
        if self
            .last_check
            .is_some_and(|last| now.saturating_duration_since(last) < POLL_INTERVAL)
        {
            return false;
        }
        self.last_check = Some(now);
        let current = stamp_of(&self.path);
        if current == self.stamp {
            return false;
        }
        let before = self.sheet.clone();
        self.reload();
        self.sheet != before
    }

    /// Writes `sheet` to disk (temp file, then rename) and makes it the current sheet.
    pub fn save(&mut self, sheet: &StyleSheet) -> Result<(), String> {
        self.write(sheet)?;
        self.sheet = sheet.clone().clamped();
        self.error = None;
        Ok(())
    }

    /// Replaces the file with the defaults.
    pub fn reset(&mut self) -> Result<(), String> {
        self.save(&StyleSheet::default())
    }

    fn reload(&mut self) {
        self.stamp = stamp_of(&self.path);
        match fs::read_to_string(&self.path) {
            Err(reason) => self.error = Some(format!("{FILE_NAME}: {reason}")),
            Ok(text) => match serde_json::from_str::<StyleSheet>(&text) {
                Ok(sheet) => {
                    self.sheet = sheet.clamped();
                    self.error = None;
                }
                Err(reason) => self.error = Some(describe(&reason)),
            },
        }
    }

    fn write(&mut self, sheet: &StyleSheet) -> Result<(), String> {
        let text = serde_json::to_string_pretty(sheet).map_err(|reason| reason.to_string())?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|reason| format!("{FILE_NAME}: {reason}"))?;
        }
        let temp = self.path.with_extension("json.tmp");
        let written = fs::write(&temp, text.as_bytes()).and_then(|()| fs::rename(&temp, &self.path));
        if let Err(reason) = written {
            let _ = fs::remove_file(&temp);
            return Err(format!("{FILE_NAME}: {reason}"));
        }
        self.stamp = stamp_of(&self.path);
        Ok(())
    }
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok(), meta.len()))
}

/// `style.json line 2: expected value`, without serde_json's trailing position.
fn describe(error: &serde_json::Error) -> String {
    let text = error.to_string();
    let message = text
        .rsplit_once(" at line ")
        .map_or(text.as_str(), |(head, _)| head);
    format!("{FILE_NAME} line {}: {message}", error.line().max(1))
}

/// Opens `path` with the operating system's default application.
pub fn open_in_default_app(path: &Path) -> Result<(), String> {
    use std::process::Command;

    #[cfg(target_os = "windows")]
    let spawned = {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up for `cmd`.
        Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .creation_flags(0x0800_0000)
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let spawned = Command::new("open").arg(path).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let spawned = Command::new("xdg-open").arg(path).spawn();

    spawned
        .map(|_| ())
        .map_err(|reason| format!("Could not open {}: {reason}", path.display()))
}
