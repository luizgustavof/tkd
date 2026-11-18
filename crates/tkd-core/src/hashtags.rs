//! Hashtags da postagem: junta as do video (escolhidas pelo agente) com as fixas do canal
//! (decididas no estudo, editaveis no app), normaliza, tira repetidas e respeita o limite.

/// Tags por post, sem contar o `#shorts` do YouTube. TikTok e YouTube punem excesso
/// (o YouTube ignora todas acima de 15; poucas e certeiras rendem mais).
pub const MAX: usize = 5;

/// `"#Jujutsu Kaisen!"` -> `"#JujutsuKaisen"`. Letras (com acento), numeros e `_`; vazio = None.
pub fn normalize(tag: &str) -> Option<String> {
    let body: String = tag
        .trim()
        .trim_start_matches('#')
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .take(50)
        .collect();
    if body.is_empty() || body.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("#{body}"))
}

/// Separa a legenda das hashtags escritas no meio dela: ("texto limpo", ["#tag", ...]).
pub fn split_caption(caption: &str) -> (String, Vec<String>) {
    let mut words = Vec::new();
    let mut tags = Vec::new();
    for w in caption.split_whitespace() {
        if w.starts_with('#') {
            tags.extend(normalize(w));
        } else {
            words.push(w);
        }
    }
    (words.join(" "), tags)
}

/// Hashtags finais de um post: as do video primeiro, depois as do canal; sem repetir
/// (sem diferenciar maiuscula), ate `MAX`. YouTube ganha `#shorts` no fim.
pub fn for_post(video: &[String], channel: &[String], platform: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in video.iter().chain(channel).filter_map(|t| normalize(t)) {
        if t.eq_ignore_ascii_case("#shorts") || out.iter().any(|o| o.to_lowercase() == t.to_lowercase()) {
            continue;
        }
        if out.len() == MAX {
            break;
        }
        out.push(t);
    }
    if platform == "youtube" {
        out.push("#shorts".into());
    }
    out
}

/// Texto + hashtags de um post, a partir da legenda e das tags do video e do canal.
pub struct Post {
    pub text: String,
    pub tags: Vec<String>,
}

impl Post {
    pub fn new(caption: &str, video_tags: &[String], channel_tags: &[String], platform: &str) -> Self {
        let (text, inline) = split_caption(caption);
        let video: Vec<String> = video_tags.iter().cloned().chain(inline).collect();
        Self { text, tags: for_post(&video, channel_tags, platform) }
    }
    /// Legenda do TikTok / descricao do YouTube: texto e tags numa linha so.
    pub fn caption(&self) -> String {
        format!("{} {}", self.text, self.tags.join(" ")).trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: &[&str]) -> Vec<String> {
        x.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize("#Jujutsu Kaisen!").as_deref(), Some("#JujutsuKaisen"));
        assert_eq!(normalize("anime").as_deref(), Some("#anime"));
        assert_eq!(normalize("##edição").as_deref(), Some("#edição"));
        assert_eq!(normalize("#").as_deref(), None);
        assert_eq!(normalize("#2024").as_deref(), None);
    }

    #[test]
    fn merges_without_repeats_and_caps() {
        let t = for_post(&v(&["#Gojo", "#jjk", "#anime"]), &v(&["#anime", "#edit", "#JJK", "#fyp", "#viral"]), "tiktok");
        assert_eq!(t, v(&["#Gojo", "#jjk", "#anime", "#edit", "#fyp"]));
        let y = for_post(&v(&["#Shorts", "#gojo"]), &v(&[]), "youtube");
        assert_eq!(y, v(&["#gojo", "#shorts"]));
    }

    #[test]
    fn strips_caption_tags() {
        let p = Post::new("Toji contra Gojo #jjk  #anime", &v(&["#anime", "#toji"]), &v(&["#edit"]), "tiktok");
        assert_eq!(p.text, "Toji contra Gojo");
        assert_eq!(p.tags, v(&["#anime", "#toji", "#jjk", "#edit"]));
        assert_eq!(p.caption(), "Toji contra Gojo #anime #toji #jjk #edit");
    }
}
