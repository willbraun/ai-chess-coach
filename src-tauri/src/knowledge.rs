use serde::Deserialize;

/// A single knowledge base document with tag-based retrieval metadata.
#[derive(Deserialize, Clone)]
pub struct KnowledgeDoc {
    pub id: String,
    pub tags: Vec<String>,
    pub title: String,
    pub content: String,
}

/// Loads all knowledge documents from the JSON files embedded at compile time.
pub fn load_knowledge() -> Vec<KnowledgeDoc> {
    let mut all: Vec<KnowledgeDoc> = Vec::new();

    for json in [
        include_str!("../knowledge/tactics.json"),
        include_str!("../knowledge/openings.json"),
        include_str!("../knowledge/middlegame.json"),
        include_str!("../knowledge/endgames.json"),
    ] {
        match serde_json::from_str::<Vec<KnowledgeDoc>>(json) {
            Ok(docs) => all.extend(docs),
            Err(e) => log::warn!("Failed to parse knowledge JSON: {}", e),
        }
    }

    all
}

/// Returns the top `max` documents whose tags appear as substrings in `findings_text`.
/// Documents with more matching tags rank higher. Documents with zero matches are excluded.
pub fn retrieve_relevant(findings_text: &str, docs: &[KnowledgeDoc], max: usize) -> Vec<KnowledgeDoc> {
    let text_lower = findings_text.to_lowercase();

    let mut scored: Vec<(usize, &KnowledgeDoc)> = docs
        .iter()
        .map(|doc| {
            let count = doc
                .tags
                .iter()
                .filter(|tag| text_lower.contains(tag.as_str()))
                .count();
            (count, doc)
        })
        .filter(|(count, _)| *count > 0)
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(max).map(|(_, doc)| doc.clone()).collect()
}
