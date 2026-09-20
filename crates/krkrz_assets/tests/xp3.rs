use flate2::{Compression, write::ZlibEncoder};
use krkrz_assets::{
    cx::CxEncryption,
    storage::Storage,
    xp3::{Archive, MAGIC, adler32},
};
use krkrz_core::Limits;
use std::io::Write;
fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut b = tag.to_vec();
    b.extend((data.len() as u64).to_le_bytes());
    b.extend(data);
    b
}
fn compressed(b: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(vec![], Compression::default());
    z.write_all(b).unwrap();
    z.finish().unwrap()
}
fn fixture(name: &str, payload: &[u8], encrypt: bool, chained: bool) -> Vec<u8> {
    let mut data = MAGIC.to_vec();
    data.extend(0u64.to_le_bytes());
    let mut encoded = payload.to_vec();
    let hash = adler32(payload);
    if encrypt {
        CxEncryption::otome_domain()
            .unwrap()
            .apply(hash, 0, &mut encoded);
    }
    let split = encoded.len() / 2;
    let mut segs = vec![];
    for (i, part) in [&encoded[..split], &encoded[split..]]
        .into_iter()
        .enumerate()
    {
        let packed = if i == 1 {
            compressed(part)
        } else {
            part.to_vec()
        };
        segs.extend((i as u32).to_le_bytes());
        segs.extend((data.len() as u64).to_le_bytes());
        segs.extend((part.len() as u64).to_le_bytes());
        segs.extend((packed.len() as u64).to_le_bytes());
        data.extend(packed);
    }
    let packed = data.len() - 19;
    let mut info = if encrypt { 0x80000000u32 } else { 0 }
        .to_le_bytes()
        .to_vec();
    info.extend((payload.len() as u64).to_le_bytes());
    info.extend((packed as u64).to_le_bytes());
    let units = name.encode_utf16().collect::<Vec<_>>();
    info.extend((units.len() as u16).to_le_bytes());
    for u in units {
        info.extend(u.to_le_bytes());
    }
    let mut file = chunk(b"info", &info);
    file.extend(chunk(b"segm", &segs));
    file.extend(chunk(b"adlr", &hash.to_le_bytes()));
    let index = chunk(b"File", &file);
    let packed = compressed(&index);
    let pos = data.len() as u64;
    data[11..19].copy_from_slice(&pos.to_le_bytes());
    if chained {
        data.push(128);
        data.extend(0u64.to_le_bytes());
        data.extend((pos + 17).to_le_bytes());
    }
    data.push(1);
    data.extend((packed.len() as u64).to_le_bytes());
    data.extend((index.len() as u64).to_le_bytes());
    data.extend(packed);
    data
}
#[test]
fn segmented_compressed_encrypted_range_reads() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("data.xp3");
    let plain = (0..80000).map(|n| (n * 17) as u8).collect::<Vec<_>>();
    std::fs::write(&path, fixture("Dir\\A.BIN", &plain, true, true)).unwrap();
    let mut a = Archive::open(
        &path,
        Limits {
            cache_bytes: 100000,
            ..Limits::default()
        },
    )
    .unwrap();
    let cx = CxEncryption::otome_domain().unwrap();
    assert!(a.read("dir/a.bin", None).is_err());
    a.verify("dir/a.bin", Some(&cx)).unwrap();
    for (start, size) in [
        (0, 80000),
        (39999, 2),
        (40000, 100),
        (70000, 1),
        (80000, 0),
        (190, 512),
    ] {
        assert_eq!(
            a.read_range("dir/a.bin", start, size, Some(&cx)).unwrap(),
            plain[start as usize..(start + size) as usize]
        );
    }
    assert!(a.read_range("dir/a.bin", 79999, 2, Some(&cx)).is_err());
}
#[test]
fn patch_overrides_and_search_paths() {
    let d = tempfile::tempdir().unwrap();
    for (archive, name, content) in [
        ("data.xp3", "scn/start.ks", b"old".as_slice()),
        ("patch2.xp3", "start.ks", b"two"),
        ("patch10.xp3", "start.ks", b"ten"),
    ] {
        std::fs::write(d.path().join(archive), fixture(name, content, false, false)).unwrap();
    }
    let mut s = Storage::open(d.path(), None, Limits::default()).unwrap();
    s.add_search_path("scn").unwrap();
    assert_eq!(s.read("start.ks").unwrap(), b"ten");
    assert_eq!(s.read("scn/start.ks").unwrap(), b"old");
    assert!(s.extract("start.ks", &d.path().join("new.ks")).is_err());
}

#[test]
fn qualified_paths_select_the_named_archive_and_roundtrip_placed_paths() {
    let d = tempfile::tempdir().unwrap();
    for (archive, content) in [
        ("data.xp3", b"original".as_slice()),
        ("patch2.xp3", b"patched"),
    ] {
        std::fs::write(
            d.path().join(archive),
            fixture("scripts/start.tjs", content, true, false),
        )
        .unwrap();
    }
    let mut storage = Storage::open(
        d.path(),
        Some(CxEncryption::otome_domain().unwrap()),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(storage.read("scripts/start.tjs").unwrap(), b"patched");
    assert_eq!(
        storage.read("DATA.XP3>Scripts/START.TJS").unwrap(),
        b"original"
    );
    let placed = storage.placed_path("data.xp3>scripts/start.tjs").unwrap();
    assert_eq!(storage.read(&placed).unwrap(), b"original");
    storage.verify(&placed).unwrap();
    assert_eq!(
        storage.read(&format!("file://.{placed}")).unwrap(),
        b"original"
    );
    storage
        .add_search_path(&format!("file://.{}/data.xp3>scripts/", d.path().display()))
        .unwrap();
    assert_eq!(storage.read("start.tjs").unwrap(), b"original");
    storage.add_search_path("patch2.xp3>scripts").unwrap();
    assert_eq!(storage.read("start.tjs").unwrap(), b"patched");
    storage.add_search_path("data.xp3>scripts").unwrap();
    assert_eq!(storage.read("start.tjs").unwrap(), b"original");
    assert!(storage.read("data.xp3>../scripts/start.tjs").is_err());
    assert!(
        storage
            .read("data.xp3>../patch2.xp3>scripts/start.tjs")
            .is_err()
    );
    assert!(storage.read("/outside/data.xp3>scripts/start.tjs").is_err());
}
#[test]
fn bad_offsets_truncation_and_budgets() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("data.xp3");
    let good = fixture("a", b"abc", false, true);
    for n in 0..good.len() {
        std::fs::write(&path, &good[..n]).unwrap();
        assert!(
            Archive::open(&path, Limits::default()).is_err(),
            "accepted truncated archive {n}"
        );
    }
    let mut bad = good.clone();
    bad[11..19].copy_from_slice(&u64::MAX.to_le_bytes());
    std::fs::write(&path, bad).unwrap();
    assert!(Archive::open(&path, Limits::default()).is_err());
    std::fs::write(&path, &good).unwrap();
    assert!(
        Archive::open(
            &path,
            Limits {
                index_bytes: 1,
                ..Limits::default()
            }
        )
        .is_err()
    );
    let mut cycle = MAGIC.to_vec();
    cycle.extend(19u64.to_le_bytes());
    cycle.push(128);
    cycle.extend(0u64.to_le_bytes());
    cycle.extend(19u64.to_le_bytes());
    std::fs::write(&path, cycle).unwrap();
    assert!(Archive::open(&path, Limits::default()).is_err());
}
#[test]
fn no_overwrite_or_path_escape() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("data.xp3");
    std::fs::write(&path, fixture("../../escape", b"abc", false, false)).unwrap();
    assert!(Archive::open(&path, Limits::default()).is_err());
    std::fs::write(&path, fixture("a", b"abc", false, false)).unwrap();
    let mut s = Storage::open(d.path(), None, Limits::default()).unwrap();
    let out = tempfile::NamedTempFile::new().unwrap();
    assert!(s.extract("a", out.path()).is_err());
}
