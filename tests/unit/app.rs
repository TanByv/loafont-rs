use super::*;

#[test]
fn default_rows_match_the_english_layout() {
    let rows = default_rows("MyFont.ttf");
    assert_eq!(rows.len(), 7);
    let keys = rows.iter().map(|row| row.key.as_str()).collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            "$YG760",
            "$YG760_12pt",
            "$YG330",
            "$YoonGasiIIM",
            "$YoonGasiIIM_Small2pt",
            "$ChronologyFont",
            "$eventDamageFont",
        ]
    );
    assert!(rows.iter().all(|row| row.file == "MyFont.ttf"));
    assert_eq!(rows[1].size, "120");
    assert_eq!(rows[3].leading, "-200");
    assert_eq!(rows[4].size, "120");
    assert_eq!(rows[4].leading, "-200");
    assert_eq!(rows[5].leading, "-200");
}

#[test]
fn unique_font_name_avoids_collisions() {
    let fonts = vec![
        FontSetting {
            path: PathBuf::from("a.ttf"),
            name: "a.ttf".to_string(),
        },
        FontSetting {
            path: PathBuf::from("b.ttf"),
            name: "a-2.ttf".to_string(),
        },
    ];
    assert_eq!(unique_font_name(Path::new("a.ttf"), &fonts), "a-3.ttf");
    assert_eq!(unique_font_name(Path::new("c.ttf"), &fonts), "c.ttf");
}

#[test]
fn removing_fonts_updates_fontmap_rows() {
    let mut rows = vec![
        FontItem {
            key: "$A".to_string(),
            file: "a.ttf".to_string(),
            ..Default::default()
        },
        FontItem {
            key: "$B".to_string(),
            file: "b.ttf".to_string(),
            ..Default::default()
        },
    ];
    let a = FontSetting {
        path: PathBuf::from("a.ttf"),
        name: "a.ttf".to_string(),
    };
    let b = FontSetting {
        path: PathBuf::from("b.ttf"),
        name: "b.ttf".to_string(),
    };

    sync_rows_to_fonts(&mut rows, &[a.clone(), b.clone()]);
    assert_eq!(rows[0].file, "a.ttf");
    assert_eq!(rows[1].file, "b.ttf");

    sync_rows_to_fonts(&mut rows, &[b]);
    assert_eq!(rows[0].file, "");
    assert_eq!(rows[1].file, "b.ttf");

    sync_rows_to_fonts(&mut rows, &[]);
    assert_eq!(rows[0].file, "");
    assert_eq!(rows[1].file, "");
}

#[test]
fn valid_font_name_rejects_paths() {
    assert!(valid_font_name("MyFont.ttf"));
    assert!(!valid_font_name(""));
    assert!(!valid_font_name(".."));
    assert!(!valid_font_name("folder/font.ttf"));
    assert!(!valid_font_name(r"folder\font.ttf"));
    assert!(!valid_font_name("C:font.ttf"));
}

#[test]
fn selects_fonts_from_the_selected_language_folder() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/font.lpk");
    let archive = Archive::read(&path, PROFILES[0]).unwrap();
    let target = lpk::normalize_path(r"\Binaries\Fonts\English");
    let fonts = archive
        .entries
        .iter()
        .filter(|entry| {
            lpk::normalize_path(entry_folder(&entry.name)) == target && is_font_file(&entry.name)
        })
        .map(|entry| entry_file_name(&entry.name))
        .collect::<Vec<_>>();

    assert_eq!(
        fonts,
        vec!["ExuberanceF-Regular.ttf", "MonarchaF-Regular.ttf"]
    );
}

#[test]
fn package_entries_use_the_selected_language_folder() {
    let font_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/Ubuntu-B.ttf");
    let fonts = vec![FontSetting {
        path: font_path.clone(),
        name: "MyFont.ttf".to_string(),
    }];
    let rows = default_rows("MyFont.ttf");
    let entries = build_entries(&fonts, &rows, r"\Binaries\Fonts\English").unwrap();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, r"\Binaries\Fonts\English\FontMap.xml");
    assert_eq!(entries[1].name, r"\Binaries\Fonts\English\MyFont.ttf");
    assert_eq!(entries[1].data, fs::read(font_path).unwrap());
}
