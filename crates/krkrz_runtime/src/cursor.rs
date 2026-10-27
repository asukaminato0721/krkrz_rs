//! Session cursor handles. Numeric values remain compatible with Layer.cursor.
use crate::{Services, Session};
use anyhow::{Context, Result, ensure};
use krkrz_assets::cursor::CursorImage;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default)]
pub(crate) struct Cursors {
    names: BTreeMap<String, i32>,
    images: BTreeMap<i32, Arc<CursorImage>>,
}
impl Services {
    pub(crate) fn load_cursor(&mut self, name: &str) -> Result<i32> {
        let path = self
            .write_storage_path(name)
            .ok()
            .filter(|path| path.is_file());
        let placed = match path {
            Some(path) => path.to_string_lossy().into_owned(),
            None => self.storage.placed_path(name)?,
        };
        let key = placed.to_ascii_lowercase();
        if let Some(id) = self.cursors.names.get(&key) {
            return Ok(*id);
        }
        ensure!(
            self.cursors.images.len() < 256,
            "cursor cache exceeds 256 images"
        );
        let image = CursorImage::decode(&self.read_storage(name)?)
            .with_context(|| format!("load cursor: {name}"))?;
        // Legacy custom cursor IDs start at 2. Positive value 1 is crHBeam.
        let id = self.cursors.images.len() as i32 + 2;
        self.cursors.images.insert(id, Arc::new(image));
        self.cursors.names.insert(key, id);
        Ok(id)
    }
}
impl Session {
    pub fn cursor_image(&self, id: i32) -> Option<&CursorImage> {
        self.services.cursors.images.get(&id).map(Arc::as_ref)
    }
}
