use sha2::{Digest, Sha256};

const SENSITIVE_KEYS: [&str; 5] = ["token", "api_key", "secret", "password", "authorization"];

pub fn redact(value: &str) -> String {
    let mut output = Vec::new();
    let mut redact_next = false;

    for token in value.split_whitespace() {
        if redact_next {
            if is_bearer_word(token) {
                output.push("<redacted>".to_owned());
                continue;
            }
            if is_separator(token) {
                output.push(token.to_owned());
                continue;
            }
            output.push("<redacted>".to_owned());
            redact_next = false;
            continue;
        }

        if let Some(has_value) = sensitive_assignment(token) {
            output.push("<redacted>".to_owned());
            redact_next = !has_value || has_embedded_authorization_scheme(token);
        } else if is_sensitive_key(token) || is_bearer_word(token) {
            output.push("<redacted>".to_owned());
            redact_next = true;
        } else if is_secret_token(token) {
            output.push("<redacted>".to_owned());
        } else {
            output.push(token.to_owned());
        }
    }

    output.join(" ")
}

fn sensitive_assignment(token: &str) -> Option<bool> {
    let token = trim_wrappers(token);
    for (index, character) in token.char_indices() {
        if !matches!(character, ':' | '=') {
            continue;
        }
        let key = trim_wrappers(&token[..index]);
        if !is_sensitive_key(key) {
            continue;
        }
        let value = trim_wrappers(&token[index + character.len_utf8()..]);
        return Some(!value.is_empty());
    }
    None
}

fn has_embedded_authorization_scheme(token: &str) -> bool {
    let token = trim_wrappers(token);
    for (index, character) in token.char_indices() {
        if !matches!(character, ':' | '=') {
            continue;
        }
        let key = trim_wrappers(&token[..index]);
        if !key.eq_ignore_ascii_case("authorization") {
            continue;
        }
        let value = trim_wrappers(&token[index + character.len_utf8()..]);
        return value.eq_ignore_ascii_case("bearer") || value.eq_ignore_ascii_case("basic");
    }
    false
}

fn is_sensitive_key(token: &str) -> bool {
    let key = trim_wrappers(token).trim_matches(|character| matches!(character, ':' | '='));
    SENSITIVE_KEYS
        .iter()
        .any(|candidate| key.eq_ignore_ascii_case(candidate))
}

fn is_bearer_word(token: &str) -> bool {
    trim_wrappers(token)
        .trim_matches(|character| matches!(character, ':' | '='))
        .eq_ignore_ascii_case("bearer")
}

fn is_secret_token(token: &str) -> bool {
    let token = trim_wrappers(token).to_ascii_lowercase();
    token.starts_with("sk-") || token.starts_with("tsk-")
}

fn is_separator(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|character| matches!(character, ':' | '=' | ',' | ';'))
}

fn trim_wrappers(value: &str) -> &str {
    value.trim_matches(|character| {
        matches!(
            character,
            '"' | '\'' | '`' | '{' | '}' | '[' | ']' | ',' | ';'
        )
    })
}

pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}
