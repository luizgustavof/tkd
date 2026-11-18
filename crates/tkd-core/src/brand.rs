//! Marca do canal: o nome com que ele assina (outro, marca d'agua, persona).
//! O estudo le perfis de outros editores; nada deles (nome, @, assinatura) pode vazar pro
//! canal. `scrub` troca qualquer mencao as referencias pela marca do canal — vale como
//! ultima barreira mesmo que o agente ignore a regra do prompt.

use crate::config::Channel;

/// Pedacos de @ que nao identificam ninguem (nao viram nome de referencia sozinhos).
const GENERIC: &[&str] = &[
    "edit", "edits", "editz", "amv", "amvs", "anime", "animes", "official", "oficial", "clips", "clip", "vfx", "fx",
    "the", "tv", "br", "fan", "fans", "page", "daily", "hd", "4k", "tiktok", "yt", "shorts", "videos", "video",
];

/// Nomes que identificam as referencias: o @ inteiro, os pedacos dele com 3+ letras que nao
/// sejam genericos e o apelido gravado no perfil (profile*.json).
pub fn reference_names(ch: &Channel) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim().trim_start_matches('@').to_string();
        if s.chars().count() >= 3 && !GENERIC.contains(&s.to_lowercase().as_str()) && !out.iter().any(|o| o.eq_ignore_ascii_case(&s)) {
            out.push(s);
        }
    };
    let mut handles = ch.ref_handles();
    // apelidos e nomes que o scrape gravou
    if let Ok(dir) = std::fs::read_dir(ch.knowledge_dir()) {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("profile") && name.ends_with(".json") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(e.path()).unwrap_or_default()) {
                    for k in ["nickname", "uniqueId", "handle", "name"] {
                        if let Some(s) = v[k].as_str() {
                            handles.push(s.to_string());
                        }
                    }
                }
            }
        }
    }
    // pedaco do @ que e assunto do canal (personagem/obra no nicho ou nas hashtags) nao e
    // nome de editor: "toji.edits" nao pode apagar "Toji" das legendas
    let subject = format!("{} {}", ch.niche, ch.hashtags.join(" ")).to_lowercase();
    let is_subject = |p: &str| {
        let p = p.to_lowercase();
        subject
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w == p)
    };
    for h in &handles {
        push(h);
        for part in h.split(|c: char| !c.is_alphanumeric()) {
            if !is_subject(part) {
                push(part);
            }
        }
    }
    // o proprio canal nunca e "referencia"
    let own = brand_of(ch).to_lowercase();
    out.retain(|n| n.to_lowercase() != own && !ch.slug.eq_ignore_ascii_case(n));
    // mais longos primeiro: "@perfil.edit" antes de "perfil"
    out.sort_by_key(|n| std::cmp::Reverse(n.chars().count()));
    out
}

/// Marca do canal: `brand` do channel.toml ou, sem ela, o 1o pedaco do nome em maiusculas
/// (anime-edits-br -> ANIME).
pub fn brand_of(ch: &Channel) -> String {
    let b = ch.brand.trim();
    if !b.is_empty() {
        return b.to_string();
    }
    ch.slug
        .split(|c: char| !c.is_alphanumeric())
        .find(|p| !p.is_empty())
        .unwrap_or("CANAL")
        .to_uppercase()
}

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Troca `name` (palavra inteira, sem diferenciar maiuscula, com ou sem @) por `brand`.
fn replace_word(text: &str, name: &str, brand: &str) -> (String, usize) {
    let lower = text.to_lowercase();
    let needle = name.to_lowercase();
    // to_lowercase pode mudar o tamanho em bytes de alguns caracteres: so seguro se nao mudou
    if lower.len() != text.len() || needle.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut n = 0;
    while let Some(pos) = lower[i..].find(&needle) {
        let start = i + pos;
        let end = start + needle.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if !is_word(before) && !is_word(after) {
            // leva o @ junto
            let cut = if before == Some('@') { start - 1 } else { start };
            out.push_str(&text[i..cut]);
            out.push_str(brand);
            n += 1;
        } else {
            out.push_str(&text[i..end]);
        }
        i = end;
    }
    out.push_str(&text[i..]);
    (out, n)
}

/// Texto sem nenhuma mencao as referencias (troca pela marca). Devolve quantas trocou.
pub fn scrub_with(text: &str, names: &[String], brand: &str) -> (String, usize) {
    let mut t = text.to_string();
    let mut total = 0;
    for name in names {
        let (nt, n) = replace_word(&t, name, brand);
        t = nt;
        total += n;
    }
    (t, total)
}

pub fn scrub(ch: &Channel, text: &str) -> (String, usize) {
    scrub_with(text, &reference_names(ch), &brand_of(ch))
}

/// Mesmo que `scrub`, em todas as strings de um JSON (menos caminhos de arquivo).
pub fn scrub_json(ch: &Channel, v: &mut serde_json::Value) -> usize {
    let names = reference_names(ch);
    let brand = brand_of(ch);
    fn walk(v: &mut serde_json::Value, names: &[String], brand: &str) -> usize {
        match v {
            serde_json::Value::String(s) => {
                let looks_path = s.contains('/') || s.contains('\\') || s.rsplit('.').next().is_some_and(|e| matches!(e, "mp4" | "png" | "jpg" | "wav" | "mp3" | "json" | "webm"));
                if looks_path {
                    return 0;
                }
                let (t, n) = scrub_with(s, names, brand);
                *s = t;
                n
            }
            serde_json::Value::Array(a) => a.iter_mut().map(|x| walk(x, names, brand)).sum(),
            serde_json::Value::Object(o) => o.values_mut().map(|x| walk(x, names, brand)).sum(),
            _ => 0,
        }
    }
    walk(v, &names, &brand)
}

/// Hashtag que cita uma referencia (#leoedit, #leo_edit): fora.
pub fn tag_mentions_reference(tag: &str, names: &[String]) -> bool {
    let t: String = tag.trim_start_matches('#').chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    names.iter().any(|n| {
        let n: String = n.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        n.chars().count() >= 3 && t.contains(&n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec!["leo_.edit".into(), "leo".into()]
    }

    #[test]
    fn swaps_reference_signature() {
        let (t, n) = scrub_with(r#"Editor que assina "LEO", inspirado em @leo_.edit."#, &names(), "ANIME");
        assert_eq!(t, r#"Editor que assina "ANIME", inspirado em ANIME."#);
        assert_eq!(n, 2);
    }

    #[test]
    fn keeps_words_containing_name() {
        let (t, n) = scrub_with("oleo, Leonardo, chameleon e leopardo", &names(), "ANIME");
        assert_eq!(t, "oleo, Leonardo, chameleon e leopardo");
        assert_eq!(n, 0);
    }

    #[test]
    fn json_keeps_paths() {
        let mut v = serde_json::json!({"caption": "by LEO", "media": "leo/clip.mp4", "scenes": [{"text": "Leo"}]});
        let names = names();
        fn walk_names(v: &mut serde_json::Value, names: &[String]) -> usize {
            match v {
                serde_json::Value::String(s) if !s.contains('/') => {
                    let (t, n) = scrub_with(s, names, "ANIME");
                    *s = t;
                    n
                }
                serde_json::Value::Array(a) => a.iter_mut().map(|x| walk_names(x, names)).sum(),
                serde_json::Value::Object(o) => o.values_mut().map(|x| walk_names(x, names)).sum(),
                _ => 0,
            }
        }
        assert_eq!(walk_names(&mut v, &names), 2);
        assert_eq!(v["media"], "leo/clip.mp4");
        assert_eq!(v["caption"], "by ANIME");
    }

    #[test]
    fn reference_hashtag() {
        assert!(tag_mentions_reference("#leoedit", &names()));
        assert!(!tag_mentions_reference("#animeedit", &names()));
    }
}
