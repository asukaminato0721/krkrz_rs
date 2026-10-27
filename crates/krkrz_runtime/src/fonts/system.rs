//! Load one system font file on demand; share its data across sessions.
use super::{FaceSelection, FontBook};
use std::sync::OnceLock;

struct SystemFont {
    book: FontBook,
    default_index: usize,
}

impl SystemFont {
    fn load(database: &fontdb::Database) -> Option<Self> {
        use fontdb::Family::{Name, SansSerif};
        // CJK fonts also cover Latin. Prefer simplified Chinese for Windows
        // names such as SimHei/黑体, which are common in translated games.
        let preferred = database.query(&fontdb::Query {
            families: &[
                Name("Noto Sans CJK SC"),
                Name("Noto Sans SC"),
                Name("Source Han Sans SC"),
                Name("WenQuanYi Zen Hei"),
                Name("WenQuanYi Micro Hei"),
                Name("Microsoft YaHei"),
                Name("SimHei"),
                Name("Noto Sans CJK JP"),
                Name("Noto Sans JP"),
                Name("Source Han Sans JP"),
                Name("Yu Gothic"),
                Name("MS Gothic"),
                SansSerif,
            ],
            ..Default::default()
        });
        preferred
            .into_iter()
            .chain(database.faces().map(|f| f.id))
            .find_map(|id| {
                database
                    .with_face_data(id, |data, index| {
                        let mut book = FontBook::default();
                        let count = book.add(data.to_vec()).ok()?;
                        (index < count).then_some(Self {
                            book,
                            default_index: index as usize,
                        })
                    })
                    .flatten()
            })
    }

    fn resolve(&self, name: &str) -> FaceSelection<'_> {
        let index = name
            .split(',')
            .find_map(|name| self.book.names.get(name.trim()))
            .copied();
        match index {
            Some(index) => FaceSelection::Named(&self.book.faces[index]),
            None => FaceSelection::Fallback(&self.book.faces[self.default_index]),
        }
    }
}

pub(super) fn resolve(name: &str) -> Option<FaceSelection<'static>> {
    static SYSTEM: OnceLock<Option<SystemFont>> = OnceLock::new();
    SYSTEM
        .get_or_init(|| {
            let mut database = fontdb::Database::new();
            database.load_system_fonts();
            SystemFont::load(&database)
        })
        .as_ref()
        .map(|font| font.resolve(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_source_can_supply_an_unregistered_windows_face() {
        let mut database = fontdb::Database::new();
        assert!(SystemFont::load(&database).is_none());
        database.load_font_data(include_bytes!("../../tests/fixtures/synthetic.ttf").to_vec());
        let system = SystemFont::load(&database).unwrap();
        let named = system.resolve("Kirikiri Synthetic");
        assert!(matches!(named, FaceSelection::Named(_)));
        assert!(matches!(system.resolve("黑体"), FaceSelection::Fallback(_)));
        assert!(std::ptr::eq(named.face(), system.resolve("黑体").face()));
        assert!(std::ptr::eq(
            named.face(),
            system.resolve("missing, Kirikiri Synthetic").face()
        ));
        assert!(
            !super::super::raster::rasterize(system.resolve("黑体").face(), 'あ', 20.0)
                .unwrap()
                .coverage
                .is_empty()
        );
    }
}
