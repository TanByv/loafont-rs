use super::*;
use crate::fontmap::{self, FontItem};
use std::time::{SystemTime, UNIX_EPOCH};

const STEAM: Profile = PROFILES[0];

fn artifact() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/font.lpk")
}

fn sample_font() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/Ubuntu-B.ttf")
}

fn temp_file(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "loa-font-test-{}-{nanos}-{name}",
        std::process::id()
    ))
}

#[test]
fn reads_the_real_archive() {
    let archive = Archive::read(&artifact(), STEAM).expect("read archive");
    assert_eq!(archive.entries.len(), 27);
    assert_eq!(
        archive.entries[0].name,
        "\\Binaries\\Fonts\\BMKkubulimTTF.ttf"
    );
    assert_eq!(archive.entries[0].size, 2_893_776);
    assert!(archive.entries.iter().all(|entry| entry.zsize != 0));
}

#[test]
fn extracts_compressed_entries() {
    let archive = Archive::read(&artifact(), STEAM).unwrap();

    let font = archive
        .entries
        .iter()
        .find(|entry| entry.name.ends_with("BMKkubulimTTF.ttf"))
        .unwrap();
    let data = archive.extract(font, STEAM).unwrap();
    assert_eq!(data.len(), font.size as usize);
    assert_eq!(&data[..4], b"\x00\x01\x00\x00");

    let fontmap = archive
        .entries
        .iter()
        .find(|entry| {
            entry
                .name
                .to_ascii_lowercase()
                .ends_with("english\\fontmap.xml")
        })
        .unwrap();
    let xml = archive.extract(fontmap, STEAM).unwrap();
    let rows = crate::fontmap::parse(&xml).unwrap();
    assert_eq!(rows.len(), 7);
    assert_eq!(rows[0].key, "$YG760");
}

#[test]
fn builds_a_package_from_scratch() {
    let font = fs::read(sample_font()).unwrap();
    let rows = vec![
        FontItem {
            key: "$YG760".to_string(),
            file: "Ubuntu-B.ttf".to_string(),
            ..Default::default()
        },
        FontItem {
            key: "$YoonGasiIIM".to_string(),
            file: "Ubuntu-B.ttf".to_string(),
            leading: "-200".to_string(),
            ..Default::default()
        },
    ];
    let fontmap = fontmap::build(&rows);

    let entries = vec![
        PackageEntry {
            name: r"\Binaries\Fonts\English\FontMap.xml".to_string(),
            data: fontmap.into_bytes(),
        },
        PackageEntry {
            name: r"\Binaries\Fonts\English\Ubuntu-B.ttf".to_string(),
            data: font.clone(),
        },
    ];

    let output = temp_file("package.lpk");
    let summary = build_package(STEAM, &entries, &output, |_| {}).unwrap();
    assert_eq!(summary.entries, 2);
    assert_eq!(fs::metadata(&output).unwrap().len(), summary.bytes);

    let archive = Archive::read(&output, STEAM).unwrap();
    assert_eq!(archive.entries.len(), 2);

    let font_entry = archive
        .entries
        .iter()
        .find(|entry| entry.name.ends_with("Ubuntu-B.ttf"))
        .unwrap();
    assert_eq!(archive.extract(font_entry, STEAM).unwrap(), font);

    let xml_entry = archive
        .entries
        .iter()
        .find(|entry| entry.name.ends_with("FontMap.xml"))
        .unwrap();
    let xml = archive.extract(xml_entry, STEAM).unwrap();
    let reparsed = fontmap::parse(&xml).unwrap();
    assert_eq!(reparsed.len(), 2);
    assert_eq!(reparsed[0].file, "Ubuntu-B.ttf");

    fs::remove_file(&output).unwrap();
}

#[test]
fn rejects_duplicate_package_entries() {
    let entries = vec![
        PackageEntry {
            name: r"\Binaries\Fonts\English\FontMap.xml".to_string(),
            data: vec![1, 2, 3, 4],
        },
        PackageEntry {
            name: r"\binaries\fonts\english\fontmap.xml".to_string(),
            data: vec![5, 6, 7, 8],
        },
    ];
    let output = temp_file("duplicate.lpk");
    assert!(build_package(STEAM, &entries, &output, |_| {}).is_err());
}

#[test]
fn wrong_profile_is_rejected() {
    let data = fs::read(artifact()).unwrap();
    let korea = Profile::from_label("Korea");
    assert!(Archive::from_bytes(&data, korea).is_err());
}
