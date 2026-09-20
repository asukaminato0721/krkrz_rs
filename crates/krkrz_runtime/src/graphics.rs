//! Decoded-image cache. Cache limits are bytes, as in GraphicsLoaderIntf.cpp.
use krkrz_assets::media::Image;
use std::{collections::VecDeque, sync::Arc};

pub struct ImageCache {
    maximum: usize,
    limit: usize,
    bytes: usize,
    entries: VecDeque<(String, Arc<Image>)>,
}

impl ImageCache {
    pub fn new(maximum: usize) -> Self {
        Self {
            maximum,
            limit: 0,
            bytes: 0,
            entries: VecDeque::new(),
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn set_limit(&mut self, value: i64) {
        // The native setter first converts to tjs_int, then to unsigned 64-bit.
        // Negative values therefore select the system maximum after clamping.
        let value = value as i32 as i64 as u64;
        self.limit = value.min(self.maximum as u64) as usize;
        self.trim();
    }

    pub fn get(&mut self, name: &str) -> Option<Arc<Image>> {
        let index = self.entries.iter().position(|(key, _)| key == name)?;
        let entry = self.entries.remove(index)?;
        let image = entry.1.clone();
        self.entries.push_front(entry);
        Some(image)
    }

    pub fn insert(&mut self, name: String, image: Arc<Image>) {
        if let Some(index) = self.entries.iter().position(|(key, _)| key == &name) {
            self.bytes -= self.entries.remove(index).unwrap().1.rgba.len();
        }
        if image.rgba.len() <= self.limit && self.limit != 0 {
            self.bytes += image.rgba.len();
            self.entries.push_front((name, image));
        }
        self.trim();
    }

    fn trim(&mut self) {
        while self.bytes > self.limit {
            let (_, image) = self.entries.pop_back().unwrap();
            self.bytes -= image.rgba.len();
        }
        if self.limit == 0 {
            self.entries.clear();
        }
    }
}

/// Kirikiri's automatic-memory policy with a 512 MiB ceiling. Linux memory
/// discovery is host-dependent; deterministic hosts can pass a fixed maximum.
pub fn automatic_limit() -> usize {
    let bytes = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("MemTotal:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()?
                    .checked_mul(1024)
            })
        })
        .unwrap_or(640 * 1024 * 1024);
    automatic_limit_for_memory(bytes)
}

fn automatic_limit_for_memory(bytes: u64) -> usize {
    const MB: u64 = 1024 * 1024;
    let megabytes = if bytes <= 64 * MB {
        0
    } else if bytes <= 96 * MB {
        4
    } else if bytes <= 128 * MB {
        8
    } else if bytes <= 192 * MB {
        12
    } else if bytes <= 256 * MB {
        20
    } else if bytes <= 512 * MB {
        40
    } else {
        bytes / (MB * 10)
    };
    (megabytes.min(512) * MB) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn eviction_preserves_active_images_and_recent_hits() {
        let image = Arc::new(Image {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        });
        let mut cache = ImageCache::new(8);
        cache.set_limit(-1);
        cache.insert("a".into(), image.clone());
        cache.insert("b".into(), image.clone());
        let active = cache.get("a").unwrap();
        cache.insert("c".into(), image.clone());
        assert!(cache.get("b").is_none());
        assert!(cache.get("a").is_some());
        cache.set_limit(0);
        assert!(cache.get("a").is_none());
        assert_eq!(active.rgba.len(), 4);
        cache.set_limit(1);
        cache.insert("large".into(), image);
        assert!(cache.get("large").is_none());
        cache.set_limit(1i64 << 32);
        assert_eq!(cache.limit(), 0);
        cache.set_limit(-2);
        assert_eq!(cache.limit(), 8);
    }
    #[test]
    fn automatic_memory_boundaries() {
        for (memory, limit) in [
            (64, 0),
            (65, 4),
            (96, 4),
            (128, 8),
            (192, 12),
            (256, 20),
            (512, 40),
            (1024, 102),
            (8192, 512),
        ] {
            assert_eq!(automatic_limit_for_memory(memory << 20), limit << 20);
        }
    }
}
