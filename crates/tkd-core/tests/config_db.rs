//! Config e estado no state.db: channel.toml com copia de seguranca, cortes.json migrado para a tabela.

#[test]
fn channel_config_and_used_clips_in_state_db() {
    let root = std::env::temp_dir().join(format!("tkd-cfgdb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("channels")).unwrap();
    tkd_core::config::set_root(&root);
    use tkd_core::config::{create_channel, edit_channel, get_channel, load_all_channels};

    create_channel("_banco", "@ref", "nicho", "").unwrap();
    let toml_path = root.join("channels/_banco/channel.toml");

    // arquivo quebrado (edicao a mao pela metade): carrega da ultima versao boa e nao sobrescreve o arquivo
    std::fs::write(&toml_path, "slug = \"_banco\"\nniche = [quebrado").unwrap();
    assert_eq!(get_channel("_banco").unwrap().niche, "nicho");
    assert!(std::fs::read_to_string(&toml_path).unwrap().contains("quebrado"));
    assert_eq!(load_all_channels().unwrap().len(), 1);

    // mudanca pela UI com o arquivo quebrado: parte da versao boa, guarda o quebrado ao lado
    let ch = edit_channel("_banco", |doc| {
        doc["niche"] = toml_edit::value("outro nicho");
        Ok(())
    })
    .unwrap();
    assert_eq!(ch.niche, "outro nicho");
    assert!(root.join("channels/_banco/channel.toml.invalid").exists());

    // arquivo apagado: recriado da copia
    std::fs::remove_file(&toml_path).unwrap();
    assert_eq!(get_channel("_banco").unwrap().niche, "outro nicho");
    assert!(toml_path.exists());

    // edicao a mao valida continua valendo
    let t = std::fs::read_to_string(&toml_path).unwrap().replace("outro nicho", "nicho a mao");
    std::fs::write(&toml_path, t).unwrap();
    assert_eq!(get_channel("_banco").unwrap().niche, "nicho a mao");

    // cortes.json antigo: migrado na primeira leitura, renomeado (nunca apagado)
    let k = root.join("channels/_banco/knowledge");
    std::fs::create_dir_all(&k).unwrap();
    let now = tkd_core::state::now();
    std::fs::write(
        k.join("cortes.json"),
        format!(r#"[{{"clip":"out/live/s/clips/clip_01.json","id":"v1","result":"postado","at":"{now}"}},
                   {{"clip":"out/live/s/clips/clip_00.json","id":"","result":"velho","at":"2020-01-01 00:00:00"}}]"#),
    )
    .unwrap();
    let ch = get_channel("_banco").unwrap();
    assert_eq!(tkd_core::clips::used_clip_paths(&ch), vec!["out/live/s/clips/clip_01.json".to_string()]);
    assert!(!k.join("cortes.json").exists());
    assert!(k.join("cortes.json.migrated").exists());
    // segunda leitura nao duplica
    assert_eq!(tkd_core::clips::used_clip_paths(&ch).len(), 1);

    // formato antigo "cortes" ainda e lido como "clips"
    let t = std::fs::read_to_string(&toml_path).unwrap();
    std::fs::write(&toml_path, format!("format = \"cortes\"
{t}")).unwrap();
    let ch = get_channel("_banco").unwrap();
    assert!(ch.is_clips());
    assert_eq!(ch.format, "clips");

    let _ = std::fs::remove_dir_all(&root);
}
