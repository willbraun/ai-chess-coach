use reqwest::Client;
use serde_json::json;

use crate::knowledge::KnowledgeDoc;

const OLLAMA_URL: &str = "http://localhost:11434/api/generate";
pub const OLLAMA_MODEL: &str = "qwen3.5";

/// Builds the full prompt for the coaching LLM call.
/// Prepends relevant knowledge docs as context, then appends the position
/// analysis text and a coaching instruction.
pub fn build_coaching_prompt(findings_text: &str, context_docs: &[KnowledgeDoc]) -> String {
    let mut prompt = String::new();

    if !context_docs.is_empty() {
        prompt.push_str("=== Chess Knowledge Reference ===\n\n");
        for doc in context_docs {
            prompt.push_str(&format!("**{}**\n{}\n\n", doc.title, doc.content));
        }
    }

    prompt.push_str("=== Position Analysis ===\n\n");
    prompt.push_str(findings_text);
    prompt.push_str("\n\n=== Coaching Task ===\n\n");
    prompt.push_str(
        "You are a chess coach. Based on the position analysis above, \
        provide concise, accurate coaching advice in 3-5 sentences. \
        Follow these strict rules:\n\
        1. MATERIAL and CHECKMATE come first. If the player lost material or missed a checkmate, \
           say so clearly and explain exactly why (e.g. a pin, a fork, an undefended piece).\n\
        2. Only describe a threat as dangerous if it actually wins material or forces checkmate. \
           Do NOT describe an attack on a defended piece as a threat — capturing a protected piece \
           at a material deficit is not a real threat.\n\
        3. Be precise about causes: if a recapture was impossible due to a pin, say so. \
           If a piece was undefended, say so.\n\
        4. Only after material/checkmate issues are addressed, mention strategic themes if relevant.",
    );

    prompt
}

/// POSTs a prompt to a locally running Ollama instance and returns the response text.
/// Returns Err if Ollama is unreachable, times out, or returns an unexpected response.
pub async fn call_ollama(prompt: String) -> Result<String, String> {
    let client = Client::builder()
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let body = json!({
        "model": OLLAMA_MODEL,
        "prompt": prompt,
        "stream": false
    });

    let response = client
        .post(OLLAMA_URL)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Ollama returned status {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

    json["response"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Missing 'response' field in Ollama response".to_string())
}
