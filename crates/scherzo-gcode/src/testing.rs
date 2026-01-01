/// Convenience helper for snapshotting parser output as pretty JSON.
pub fn snapshot_from_str(input: &str) -> String {
    match crate::parse(input) {
        Ok(statements) => serde_json::to_string_pretty(&statements)
            .unwrap_or_else(|err| format!("failed to render JSON: {err}")),
        Err(err) => format!("parse error: {err}"),
    }
}

/// Convenience helper for snapshotting lexer output as pretty JSON.
pub fn snapshot_tokens_from_str(input: &str) -> String {
    let tokens: Result<Vec<_>, _> = crate::lex(input).collect();
    match tokens {
        Ok(tokens) => serde_json::to_string_pretty(&tokens)
            .unwrap_or_else(|err| format!("failed to render JSON: {err}")),
        Err(err) => format!("lex error: {err}"),
    }
}
