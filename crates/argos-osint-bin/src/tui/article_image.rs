//! Bounded background article imagery. Draw only consumes encoded protocols.
use image::{DynamicImage, ImageReader, Limits};
use ratatui::layout::{Rect, Size};
use ratatui_image::{picker::Picker, protocol::Protocol, Image, Resize};
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
                frame.render_widget(Image::new(protocol), area);
                return;
            }
        }
        frame.render_widget(
            ratatui::widgets::Paragraph::new("Loading image…").style(super::theme::muted()),
            area,
        );
    }
}

fn encode(picker: &Picker, image: &DynamicImage, size: Size) -> Option<Protocol> {
    let font = picker.font_size();
    let width = (u32::from(size.width) * u32::from(font.width)).clamp(1, 2048);
    let height = (u32::from(size.height) * u32::from(font.height)).clamp(1, 2048);
    let contained = image.resize(width, height, image::imageops::FilterType::Triangle);
    picker.new_protocol(contained, size, Resize::Fit(None)).ok()
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
    reader
        .decode()
        .ok()
        .map(|image| image.thumbnail(2048, 2048))
}

#[cfg(test)]
mod tests {
    use super::*;
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
