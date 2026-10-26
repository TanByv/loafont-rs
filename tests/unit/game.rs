use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("loa-font-game-test-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn mod_state_prefers_the_recorded_hash() {
    let dir = temp_dir();
    let paths = paths(&dir);
    fs::write(&paths.font, b"original").unwrap();

    assert_eq!(current_is_modded(&dir, None), None);
    assert_eq!(current_is_modded(&dir, Some(false)), None);
    assert_eq!(current_is_modded(&dir, Some(true)), Some(true));

    let info = ModInfo {
        mod_sha256: sha256_file(&paths.font).unwrap(),
    };
    fs::write(&paths.info, serde_json::to_vec(&info).unwrap()).unwrap();
    assert_eq!(current_is_modded(&dir, Some(false)), Some(true));

    fs::write(&paths.font, b"updated by steam").unwrap();
    assert_eq!(current_is_modded(&dir, Some(true)), Some(false));

    fs::remove_dir_all(&dir).unwrap();
}
