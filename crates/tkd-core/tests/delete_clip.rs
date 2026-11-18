//! Apagar corte pela tela "Cortes": so os arquivos do corte, nunca vizinho nem nada fora de out/live.

#[test]
fn deletes_only_the_right_clip() {
    let root = std::env::temp_dir().join(format!("tkd-del-{}", std::process::id()));
    let clips = root.join("out/live/twitch-x-20261118-0900/clips");
    std::fs::create_dir_all(&clips).unwrap();
    for f in ["clip_1.json", "clip_1.mp4", "clip_1_9x16.mp4", "clip_1.srt", "clip_10.json", "clip_10.mp4"] {
        std::fs::write(clips.join(f), b"x").unwrap();
    }
    std::fs::write(root.join("outro.json"), b"x").unwrap();
    tkd_core::config::set_root(&root);

    assert_eq!(tkd_core::clips::delete_clip("out/live/twitch-x-20261118-0900/clips/clip_1.json").unwrap(), 4);
    let mut left: Vec<String> = std::fs::read_dir(&clips).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into()).collect();
    left.sort();
    assert_eq!(left, ["clip_10.json", "clip_10.mp4"]);

    assert!(tkd_core::clips::delete_clip("outro.json").is_err(), "fora de out/live");
    assert!(tkd_core::clips::delete_clip("out/live/twitch-x-20261118-0900/clips/../../../../outro.json").is_err());
    assert!(tkd_core::clips::delete_clip("out/live/twitch-x-20261118-0900/clips/clip_10.mp4").is_err(), "so .json");
    assert!(root.join("outro.json").exists());
    let _ = std::fs::remove_dir_all(&root);
}
