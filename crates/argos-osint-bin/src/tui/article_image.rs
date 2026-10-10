//! Bounded background article imagery. Draw only consumes encoded protocols.
use image::{DynamicImage, ImageReader, Limits};
use ratatui::layout::{Rect, Size};
use ratatui_image::{
    picker::{Picker, ProtocolType},
    protocol::Protocol,
    Image, Resize,
};
use std::{
    collections::HashMap,
    io::Cursor,
    sync::{mpsc, Arc},
    time::Duration,
};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE: usize = 8;
type Key = (String, String);
enum Entry {
    Loading,
    Ready(Arc<DynamicImage>),
    Failed,
}
enum Completion {
    Decoded(Key, Option<DynamicImage>),
    Encoded(Key, Size, Option<Protocol>),
}

pub struct ArticleImages {
    picker: Picker,
    cache: HashMap<Key, Entry>,
    tx: mpsc::Sender<Completion>,
    rx: mpsc::Receiver<Completion>,
    encoded: Option<(Key, Size, Protocol)>,
    encoding: Option<(Key, Size)>,
    active: Option<(Key, Size)>,
    pub clear_graphics: bool,
}
impl Default for ArticleImages {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            picker: Picker::halfblocks(),
            cache: HashMap::new(),
            tx,
            rx,
            encoded: None,
            encoding: None,
            active: None,
            clear_graphics: false,
        }
    }
}
impl ArticleImages {
    /// Called once in the terminal event loop, after alternate-screen entry.
    pub fn detect(&mut self) {
        self.picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        let protocol = terminal_protocol(
            self.picker.protocol_type(),
            &std::env::var("TERM_PROGRAM").unwrap_or_default(),
            &std::env::var("TERM").unwrap_or_default(),
            std::env::var_os("TMUX").is_some() || std::env::var_os("STY").is_some(),
        );
        self.picker.set_protocol_type(protocol);
    }

    #[cfg(test)]
    pub fn fixture(&mut self, id: &str, url: &str, area: Rect) {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 32, |x, y| {
            image::Rgb([(x * 4) as u8, (y * 8) as u8, 120])
        }));
        let protocol = encode(
            &Picker::halfblocks(),
            &image,
            Size::new(area.width, area.height),
        )
        .unwrap();
        let key = (id.into(), url.into());
        self.cache
            .insert(key.clone(), Entry::Ready(Arc::new(image)));
        self.encoded = Some((key, Size::new(area.width, area.height), protocol));
    }

    pub fn visible(&self, id: &str, url: &str) -> bool {
        valid_url(url)
            && !matches!(
                self.cache.get(&(id.into(), url.into())),
                Some(Entry::Failed)
            )
    }

    /// Poll, fetch and encode in the controller, never in draw.
    pub fn update(&mut self, article: Option<(&str, &str)>, area: Rect) -> bool {
        let mut changed = false;
        while let Ok(completion) = self.rx.try_recv() {
            match completion {
                Completion::Decoded(key, image) => {
                    self.cache.insert(
                        key,
                        image.map_or(Entry::Failed, |image| Entry::Ready(Arc::new(image))),
                    );
                    changed = true;
                }
                Completion::Encoded(key, size, protocol) => {
                    if self.encoding.as_ref() == Some(&(key.clone(), size)) {
                        self.encoding = None;
                    }
                    if self.active.as_ref() == Some(&(key.clone(), size)) {
                        if protocol.is_none() {
                            self.cache.insert(key.clone(), Entry::Failed);
                        }
                        self.encoded = protocol.map(|protocol| (key, size, protocol));
                        changed = true;
                    }
                }
            }
        }
        let active = article
            .filter(|(_, url)| valid_url(url))
            .filter(|_| area.width > 0 && area.height > 0)
            .map(|(id, url)| {
                (
                    (id.to_owned(), url.to_owned()),
                    Size::new(area.width, area.height),
                )
            });
        if active != self.active {
            self.active = active.clone();
            self.encoded = None;
            self.clear_graphics = true;
            changed = true;
        }
        let Some((key, size)) = active else {
            return changed;
        };
        if !self.cache.contains_key(&key) {
            if self
                .cache
                .values()
                .filter(|entry| matches!(entry, Entry::Loading))
                .count()
                >= 4
            {
                return changed;
            }
            if self.cache.len() >= MAX_CACHE {
                if let Some(old) = self
                    .cache
                    .iter()
                    .find(|(id, entry)| **id != key && !matches!(entry, Entry::Loading))
                    .map(|(key, _)| key.clone())
                {
                    self.cache.remove(&old);
                }
            }
            self.cache.insert(key.clone(), Entry::Loading);
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let data = fetch(&key.1).await;
                let image = match data {
                    Some(data) => tokio::task::spawn_blocking(move || decode(data))
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                };
                let _ = tx.send(Completion::Decoded(key, image));
            });
            return true;
        }
        if self.encoded.is_none() && self.encoding.is_none() {
            if let Some(Entry::Ready(image)) = self.cache.get(&key) {
                let image = image.clone();
                let picker = self.picker.clone();
                let tx = self.tx.clone();
                self.encoding = Some((key.clone(), size));
                tokio::task::spawn_blocking(move || {
                    let protocol = encode(&picker, &image, size);
                    let _ = tx.send(Completion::Encoded(key, size, protocol));
                });
            }
        }
        changed
    }

    pub fn draw(&self, frame: &mut ratatui::Frame, area: Rect, id: &str, url: &str) {
        if let Some((key, size, protocol)) = &self.encoded {
            if key.0 == id && key.1 == url && size.width <= area.width && size.height <= area.height
            {
                frame.render_widget(
                    Image::new(protocol),
                    centered_image_rect(area, protocol.size()),
                );
                return;
            }
        }
        frame.render_widget(
            ratatui::widgets::Paragraph::new("Loading image…").style(super::theme::muted()),
            area,
        );
    }
}

/// Queries win. Direct-session hints also work when pixel/font queries are
/// unavailable; the picker's default cell aspect ratio is then used for sizing.
/// Never trust outer-terminal hints through a multiplexer.
fn terminal_protocol(
    detected: ProtocolType,
    program: &str,
    term: &str,
    multiplexed: bool,
) -> ProtocolType {
    if detected != ProtocolType::Halfblocks || multiplexed {
        return detected;
    }
    match program.to_ascii_lowercase().as_str() {
        "ghostty" | "kitty" => ProtocolType::Kitty,
        "iterm.app" | "iterm2" | "vscode" => ProtocolType::Iterm2,
        "apple_terminal" => ProtocolType::Halfblocks,
        _ if matches!(term, "xterm-kitty" | "xterm-ghostty") => ProtocolType::Kitty,
        _ => detected,
    }
}

fn encode(picker: &Picker, image: &DynamicImage, size: Size) -> Option<Protocol> {
    // Keep the decoded source intact and perform one high-quality contain resize
    // for the terminal's actual pixel dimensions, including HiDPI cell sizes.
    picker
        .new_protocol(
            image.clone(),
            size,
            Resize::Scale(Some(image::imageops::FilterType::Lanczos3)),
        )
        .ok()
}

fn centered_image_rect(area: Rect, size: Size) -> Rect {
    let width = size.width.min(area.width);
    let height = size.height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn valid_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}
async fn fetch(url: &str) -> Option<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(4))
        .build()
        .ok()?;
    let mut response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return None;
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if data.len().saturating_add(chunk.len()) > MAX_BYTES {
            return None;
        }
        data.extend_from_slice(&chunk);
    }
    Some(data)
}
fn decode(data: Vec<u8>) -> Option<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_detection_preserves_queries_and_covers_direct_session_hints() {
        for (program, term, expected) in [
            ("ghostty", "xterm-256color", ProtocolType::Kitty),
            ("kitty", "xterm-kitty", ProtocolType::Kitty),
            ("iTerm.app", "xterm-256color", ProtocolType::Iterm2),
            ("vscode", "xterm-256color", ProtocolType::Iterm2),
            ("Apple_Terminal", "xterm-256color", ProtocolType::Halfblocks),
            ("", "xterm-kitty", ProtocolType::Kitty),
            ("unknown", "xterm-256color", ProtocolType::Halfblocks),
        ] {
            assert_eq!(
                terminal_protocol(ProtocolType::Halfblocks, program, term, false),
                expected
            );
            assert_eq!(
                terminal_protocol(ProtocolType::Sixel, program, term, false),
                ProtocolType::Sixel
            );
            assert_eq!(
                terminal_protocol(ProtocolType::Halfblocks, program, term, true),
                ProtocolType::Halfblocks
            );
        }
    }
    #[test]
    fn native_encoders_keep_the_selected_graphics_protocol() {
        let image = DynamicImage::new_rgb8(64, 32);
        for kind in [
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
            ProtocolType::Sixel,
        ] {
            let mut picker = Picker::halfblocks();
            picker.set_protocol_type(kind);
            let protocol = encode(&picker, &image, Size::new(8, 4)).unwrap();
            assert!(matches!(
                (kind, protocol),
                (ProtocolType::Kitty, Protocol::Kitty(_))
                    | (ProtocolType::Iterm2, Protocol::ITerm2(_))
                    | (ProtocolType::Sixel, Protocol::Sixel(_))
            ));
        }
    }

    #[test]
    fn decode_preserves_source_resolution_and_pixels() {
        let source = DynamicImage::ImageRgb8(image::RgbImage::from_fn(2500, 2, |x, _| {
            image::Rgb([(x % 256) as u8, 50, 200])
        }));
        let mut bytes = Cursor::new(Vec::new());
        source
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode(bytes.into_inner()).unwrap();
        assert_eq!(decoded.to_rgb8(), source.to_rgb8());
    }

    #[test]
    fn image_is_centered_in_both_axes_without_stretching() {
        assert_eq!(
            centered_image_rect(Rect::new(20, 5, 80, 20), Size::new(40, 10)),
            Rect::new(40, 10, 40, 10)
        );
        let protocol = encode(
            &Picker::halfblocks(),
            &DynamicImage::new_rgb8(640, 320),
            Size::new(80, 10),
        )
        .unwrap();
        assert_eq!(protocol.size(), Size::new(40, 10));
    }
    #[test]
    fn absent_invalid_failed_images_collapse() {
        let mut images = ArticleImages::default();
        assert!(!images.visible("a", ""));
        assert!(!images.visible("a", "file:///secret"));
        images.cache.insert(
            ("a".into(), "https://example.com/a.png".into()),
            Entry::Failed,
        );
        assert!(!images.visible("a", "https://example.com/a.png"));
        assert!(decode(vec![0; 10]).is_none());
    }
    #[test]
    fn stale_encoding_cannot_replace_active_article() {
        let mut images = ArticleImages::default();
        let picker = Picker::halfblocks();
        let protocol = picker
            .new_protocol(
                DynamicImage::new_rgb8(4, 4),
                Size::new(4, 4),
                Resize::Fit(None),
            )
            .unwrap();
        images
            .tx
            .send(Completion::Encoded(
                ("old".into(), "https://example.com/old.png".into()),
                Size::new(4, 4),
                Some(protocol),
            ))
            .unwrap();
        images.update(None, Rect::default());
        assert!(images.encoded.is_none());
    }
}
