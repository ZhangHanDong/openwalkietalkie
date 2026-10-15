//! Bounded, local image previews. No remote URL fetching and no original bytes
//! in model state, events or logs. Only typed, public image blocks are accepted.
use base64::{engine::general_purpose::STANDARD, Engine};
use deck_model::ChatImage;
use image::{GenericImageView, ImageDecoder, ImageReader};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io::{Cursor, Read},
    sync::{Arc, Mutex, OnceLock},
};

const SOURCE_LIMIT: usize = 8 * 1024 * 1024;
const CACHE_LIMIT: usize = 8 * 1024 * 1024;
const ENTRY_LIMIT: usize = 64;
pub const CHUNK: usize = 384;
pub struct Preview {
    pub meta: ChatImage,
    pub png: Vec<u8>,
    pub deck: Vec<u8>, // 256 little-endian RGB565 palette entries, then indices
    pub width: u16,
    pub height: u16,
}
#[derive(Default)]
struct Cache {
    entries: VecDeque<Arc<Preview>>,
    bytes: usize,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
// Parsing several active sessions must not multiply decoder allocations.
static DECODE: Mutex<()> = Mutex::new(());
fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(Mutex::default)
}
pub fn get(id: &str) -> Option<Arc<Preview>> {
    let mut c = cache().lock().ok()?;
    let i = c.entries.iter().position(|p| p.meta.id == id)?;
    let p = c.entries.remove(i)?;
    c.entries.push_back(p.clone());
    Some(p)
}

fn ingest(bytes: &[u8], label: &str) -> Option<ChatImage> {
    if bytes.is_empty() || bytes.len() > SOURCE_LIMIT {
        return None;
    }
    let id = blake3::hash(bytes).to_hex().to_string();
    if let Some(p) = get(&id) {
        return Some(p.meta.clone());
    }
    let _decode = DECODE.lock().ok()?;
    if let Some(p) = get(&id) {
        return Some(p.meta.clone());
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok()?;
    let mut decoded = image::DynamicImage::from_decoder(decoder).ok()?;
    decoded.apply_orientation(orientation);
    let (width, height) = decoded.dimensions();
    // Flatten transparency onto the app/device's neutral background.
    let mut small = decoded.thumbnail(128, 128).to_rgba8();
    for p in small.pixels_mut() {
        let a = u16::from(p[3]);
        for channel in &mut p.0[..3] {
            *channel = ((u16::from(*channel) * a + 24 * (255 - a)) / 255) as u8;
        }
        p[3] = 255;
    }
    let quant = color_quant::NeuQuant::new(10, 256, small.as_raw());
    let mut deck = Vec::with_capacity(512 + small.len() / 4);
    for c in quant.color_map_rgb().chunks_exact(3) {
        let rgb =
            ((u16::from(c[0]) >> 3) << 11) | ((u16::from(c[1]) >> 2) << 5) | (u16::from(c[2]) >> 3);
        deck.extend_from_slice(&rgb.to_le_bytes());
    }
    for p in small.pixels() {
        deck.push(quant.index_of(&p.0) as u8);
    }
    let mut png = Cursor::new(Vec::new());
    decoded
        .thumbnail(1024, 1024)
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()?;
    let png = png.into_inner();
    if png.len() > 4 * 1024 * 1024 {
        return None;
    }
    let meta = ChatImage {
        id,
        label: deck_model::sanitize::cut(label, 96),
        width,
        height,
    };
    let preview = Arc::new(Preview {
        meta: meta.clone(),
        png,
        deck,
        width: small.width() as u16,
        height: small.height() as u16,
    });
    let size = preview.png.len() + preview.deck.len();
    let mut c = cache().lock().ok()?;
    // Another parser may have completed this same image concurrently.
    if c.entries.iter().any(|p| p.meta.id == meta.id) {
        return Some(meta);
    }
    while c.bytes + size > CACHE_LIMIT || c.entries.len() >= ENTRY_LIMIT {
        let old = c.entries.pop_front()?;
        c.bytes -= old.png.len() + old.deck.len();
    }
    c.bytes += size;
    c.entries.push_back(preview);
    Some(meta)
}

fn encoded(data: &str, label: &str) -> Option<ChatImage> {
    let data = if data.starts_with("data:image/") {
        let (header, body) = data.split_once(',')?;
        if !header.ends_with(";base64") {
            return None;
        }
        body
    } else {
        data
    };
    if data.len() > SOURCE_LIMIT * 4 / 3 + 4 {
        return None;
    }
    ingest(&STANDARD.decode(data).ok()?, label)
}
fn local(path: &str) -> Option<ChatImage> {
    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let info = file.metadata().ok()?;
    if !info.is_file() || info.len() > SOURCE_LIMIT as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(SOURCE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    ingest(&bytes, path.file_name()?.to_str().unwrap_or("Image"))
}
pub fn has_images(content: &Value) -> bool {
    content.as_array().into_iter().flatten().any(|b| {
        matches!(
            b["type"].as_str(),
            Some("image" | "input_image" | "inputImage" | "localImage")
        )
    })
}

/// Direct content only: never recurse through thinking, tool arguments or text.
pub fn blocks(content: &Value) -> Vec<ChatImage> {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| match b["type"].as_str()? {
            "image" | "input_image" | "inputImage" => {
                if b["source"]["type"] == "base64" {
                    encoded(b["source"]["data"].as_str()?, "Image")
                } else if let Some(data) = b["data"].as_str() {
                    encoded(data, "Image")
                } else {
                    let url = b["url"]
                        .as_str()
                        .or_else(|| b["image_url"].as_str())
                        .or_else(|| b["imageUrl"].as_str())?;
                    url.starts_with("data:image/")
                        .then(|| encoded(url, "Image"))
                        .flatten()
                }
            }
            "localImage" => local(b["path"].as_str()?),
            _ => None,
        })
        .take(4)
        .collect()
}
pub fn codex(item: &Value) -> Vec<ChatImage> {
    match item["type"].as_str().unwrap_or_default() {
        "imageView" => local(item["path"].as_str().unwrap_or_default())
            .into_iter()
            .collect(),
        "imageGeneration" => item["savedPath"]
            .as_str()
            .and_then(local)
            .or_else(|| {
                encoded(
                    item["result"].as_str().unwrap_or_default(),
                    "Generated image",
                )
            })
            .into_iter()
            .collect(),
        "mcpToolCall" => blocks(&item["result"]["content"]),
        "dynamicToolCall" => {
            let mut out = blocks(&item["contentItems"]);
            out.extend(blocks(&item["result"]["content"]));
            out.truncate(4);
            out
        }
        "userMessage" | "user_message" => blocks(&item["content"]),
        _ => Vec::new(),
    }
}

#[cfg(test)]
pub(crate) fn fixture() -> String {
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(16, 9)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    STANDARD.encode(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_images_are_bounded_and_decodable() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(16, 9)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let data = STANDARD.encode(png.into_inner());
        let images =
            blocks(&serde_json::json!([{"type":"image","source":{"type":"base64","data":data}}]));
        assert_eq!(images.len(), 1);
        let p = get(&images[0].id).unwrap();
        assert_eq!((p.meta.width, p.meta.height), (16, 9));
        assert_eq!(p.deck.len(), 512 + p.width as usize * p.height as usize);
        assert!(p.width <= 128 && p.height <= 128);
        assert!(image::load_from_memory(&p.png).is_ok());
        assert!(blocks(&serde_json::json!([{"type":"thinking","data":data},{"type":"image","url":"https://example.com/a.png"}])).is_empty());
        assert!(encoded("broken base64", "Image").is_none());
    }

    #[test]
    fn image_sources_and_size_limits() {
        let data = fixture();
        for block in [
            serde_json::json!({"type":"image","data":data,"mimeType":"image/png"}),
            serde_json::json!({"type":"image","url":format!("data:image/png;base64,{data}")}),
            serde_json::json!({"type":"inputImage","imageUrl":format!("data:image/png;base64,{data}")}),
        ] {
            assert_eq!(blocks(&serde_json::json!([block])).len(), 1);
        }
        let images = codex(&serde_json::json!({"type":"imageGeneration","result":data}));
        assert_eq!(images.len(), 1);
        assert!(encoded(&"A".repeat(SOURCE_LIMIT * 4 / 3 + 8), "large").is_none());
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(8193, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        assert!(ingest(&png.into_inner(), "too wide").is_none());
    }
}
