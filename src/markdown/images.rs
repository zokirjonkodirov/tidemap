//! Local images on the rendered page: decoded off the UI thread, uploaded once, shown scaled.
//!
//! The UI thread never reads or decodes a file. A request goes to a small pool of worker
//! threads; the decoded (and, when huge, downscaled) pixels come back over a channel and become
//! a texture the next time the page asks. Remote images are not fetched: the app is offline.

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender, channel},
};

use egui::{ColorImage, Context, TextureHandle, TextureOptions, Vec2};

/// Images wider than this are shrunk while decoding; the screen never needs more.
const MAX_PIXELS: u32 = 2400;
const MAX_FILE_BYTES: u64 = 64 << 20;
const WORKERS: usize = 3;
/// Textures kept at once; the least recently shown go first.
const KEEP: usize = 48;

pub enum Entry {
    Loading,
    Ready { texture: TextureHandle, size: Vec2, used: u64 },
    Failed(String),
}

type Decoded = Result<(ColorImage, Vec2), String>;

pub struct Images {
    entries: HashMap<PathBuf, Entry>,
    queue: VecDeque<PathBuf>,
    running: usize,
    tx: Sender<(PathBuf, Decoded)>,
    rx: Receiver<(PathBuf, Decoded)>,
    frame: u64,
}

impl Default for Images {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { entries: HashMap::new(), queue: VecDeque::new(), running: 0, tx, rx, frame: 0 }
    }
}

/// The file an image source points to, when it is local. `dir` is the folder of the document.
pub fn resolve(dir: Option<&Path>, src: &str) -> Option<PathBuf> {
    let src = src.trim();
    if src.is_empty() || src.starts_with("http://") || src.starts_with("https://") || src.starts_with("data:") {
        return None;
    }
    let src = src.strip_prefix("file:///").or_else(|| src.strip_prefix("file://")).unwrap_or(src);
    let src = src.split(['?', '#']).next().unwrap_or(src);
    let decoded = percent_decode(src);
    let path = PathBuf::from(decoded.replace('/', std::path::MAIN_SEPARATOR_STR));
    if path.is_absolute() { Some(path) } else { dir.map(|dir| dir.join(path)) }
}

fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn decode(path: &Path) -> Decoded {
    let meta = std::fs::metadata(path).map_err(|_| "File not found".to_owned())?;
    if meta.len() > MAX_FILE_BYTES {
        return Err("Image is too large".into());
    }
    if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("svg")) {
        return Err("SVG images are not drawn".into());
    }
    let image = image::open(path).map_err(|_| "Unsupported or damaged image".to_owned())?;
    let natural = Vec2::new(image.width() as f32, image.height() as f32);
    let image = if image.width() > MAX_PIXELS || image.height() > MAX_PIXELS {
        image.thumbnail(MAX_PIXELS, MAX_PIXELS)
    } else {
        image
    };
    let rgba = image.into_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok((ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), natural))
}

impl Images {
    /// Collects finished decodes and starts queued ones. Call once per frame.
    pub fn poll(&mut self, ctx: &Context) {
        self.frame += 1;
        while let Ok((path, result)) = self.rx.try_recv() {
            self.running = self.running.saturating_sub(1);
            let entry = match result {
                Ok((pixels, natural)) => {
                    let texture = ctx.load_texture(path.to_string_lossy(), pixels, TextureOptions::LINEAR);
                    Entry::Ready { texture, size: natural, used: self.frame }
                }
                Err(reason) => Entry::Failed(reason),
            };
            self.entries.insert(path, entry);
            ctx.request_repaint();
        }
        while self.running < WORKERS {
            let Some(path) = self.queue.pop_front() else { break };
            self.running += 1;
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = decode(&path);
                let _ = tx.send((path, result));
                ctx.request_repaint();
            });
        }
        if self.entries.len() > KEEP {
            let mut ready: Vec<(u64, PathBuf)> = self
                .entries
                .iter()
                .filter_map(|(path, entry)| match entry {
                    Entry::Ready { used, .. } => Some((*used, path.clone())),
                    _ => None,
                })
                .collect();
            ready.sort();
            let extra = self.entries.len() - KEEP;
            for (_, path) in ready.into_iter().take(extra) {
                self.entries.remove(&path);
            }
        }
    }

    /// The state of `path`, starting its load on first sight.
    pub fn get(&mut self, path: &Path) -> &Entry {
        if !self.entries.contains_key(path) {
            self.entries.insert(path.to_owned(), Entry::Loading);
            self.queue.push_back(path.to_owned());
        }
        let frame = self.frame;
        let entry = self.entries.get_mut(path).expect("just inserted");
        if let Entry::Ready { used, .. } = entry {
            *used = frame;
        }
        entry
    }

    /// Forgets failures so a file that appeared since can load.
    pub fn retry_failed(&mut self) {
        self.entries.retain(|_, entry| !matches!(entry, Entry::Failed(_)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_local_sources_only() {
        let dir = Path::new("docs");
        assert_eq!(resolve(Some(dir), "a%20b.png"), Some(dir.join("a b.png")));
        assert_eq!(resolve(Some(dir), "img/x.png?raw=1"), Some(dir.join("img").join("x.png")));
        assert_eq!(resolve(Some(dir), "https://x.io/a.png"), None);
        assert_eq!(resolve(Some(dir), "data:image/png;base64,AA"), None);
    }

    #[test]
    fn decodes_a_png_off_thread() {
        let dir = std::env::temp_dir().join("tidemap-image-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 255])).save(&path).unwrap();
        let ctx = Context::default();
        let mut images = Images::default();
        assert!(matches!(images.get(&path), Entry::Loading));
        for _ in 0..200 {
            images.poll(&ctx);
            if matches!(images.get(&path), Entry::Ready { .. }) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let Entry::Ready { size, .. } = images.get(&path) else { panic!("image never loaded") };
        assert_eq!((size.x, size.y), (8.0, 4.0));
    }
}
