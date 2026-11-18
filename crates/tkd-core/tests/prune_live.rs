//! Limpeza local de out/live (substitui a do backup na nuvem): so sessao parada ha mais dos dias pedidos sai.

use std::time::{Duration, SystemTime};

fn age(path: &std::path::Path, days: u64) {
    let t = SystemTime::now() - Duration::from_secs(days * 86_400);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
}

#[test]
fn prunes_only_stale_sessions() {
    let root = std::env::temp_dir().join(format!("tkd-prune-{}", std::process::id()));
    let live = root.join("out/live");
    for s in ["velha", "nova", "mista"] {
        std::fs::create_dir_all(live.join(s).join("clips")).unwrap();
    }
    let old = live.join("velha/clips/clip_01.mp4");
    std::fs::write(&old, b"x").unwrap();
    age(&old, 10);
    std::fs::write(live.join("nova/clips/clip_01.mp4"), b"x").unwrap();
    // sessao com um arquivo velho e outro recente (chat ainda gravando) fica
    let m_old = live.join("mista/clips/clip_01.mp4");
    std::fs::write(&m_old, b"x").unwrap();
    age(&m_old, 10);
    std::fs::write(live.join("mista/chat.jsonl"), b"x").unwrap();
    tkd_core::config::set_root(&root);

    assert_eq!(tkd_core::live::prune_old_sessions(0), 0, "0 = nunca apaga");
    assert_eq!(tkd_core::live::prune_old_sessions(3), 1);
    assert!(!live.join("velha").exists());
    assert!(live.join("nova").exists() && live.join("mista").exists());
    let _ = std::fs::remove_dir_all(&root);
}
