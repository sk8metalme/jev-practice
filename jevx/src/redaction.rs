use std::path::{Component, Path};

use sha2::{Digest, Sha256};

const SENSITIVE_KEYS: [&str; 5] = ["token", "api_key", "secret", "password", "authorization"];
const SENSITIVE_KEY_PARTS: [&str; 6] = [
    "secret",
    "password",
    "passwd",
    "credential",
    "credentials",
    "authorization",
];

pub fn redact(value: &str) -> String {
    let mut output = Vec::new();
    let mut inside_private_key = false;
    let mut redact_next_line = false;
    let mut redact_authorization_scheme = false;
    let mut continuation_indent = None;
    let mut continuation_started = false;

    for line in value.lines() {
        if let Some(anchor_indent) = continuation_indent {
            if line.trim().is_empty() {
                output.push(String::new());
                continue;
            }

            if leading_whitespace(line) > anchor_indent {
                output.push("<redacted>".to_owned());
                continuation_started = true;
                continue;
            }

            continuation_indent = None;
            if continuation_started {
                redact_next_line = false;
                redact_authorization_scheme = false;
            }
            continuation_started = false;
        }

        if redact_next_line {
            let mut tokens = line.split_whitespace();
            let Some(first) = tokens.next() else {
                output.push(String::new());
                continue;
            };
            let second = tokens.next();
            if second.is_none() && (is_separator(first) || is_redaction_connector(first)) {
                output.push(first.to_owned());
                continue;
            }
            if second.is_none()
                && (is_bearer_word(first)
                    || (redact_authorization_scheme && is_auth_scheme_word(first)))
            {
                output.push("<redacted>".to_owned());
                continue;
            }
            if is_redaction_connector(first) {
                output.push(format!("{first} <redacted>"));
            } else {
                output.push("<redacted>".to_owned());
            }
            redact_next_line = false;
            redact_authorization_scheme = false;
            continue;
        }
        if inside_private_key {
            output.push("<redacted>".to_owned());
            inside_private_key = !is_private_key_end(line);
            redact_next_line = false;
            redact_authorization_scheme = false;
            continue;
        }
        if is_private_key_begin(line) {
            output.push("<redacted>".to_owned());
            inside_private_key = !is_private_key_end(line);
            redact_next_line = false;
            redact_authorization_scheme = false;
            continue;
        }
        let (redacted, next_line, authorization_scheme, redact_continuation) =
            redact_line(line, redact_next_line, redact_authorization_scheme);
        output.push(redacted);
        redact_next_line = next_line;
        redact_authorization_scheme = authorization_scheme;
        continuation_indent = redact_continuation.then(|| leading_whitespace(line));
    }
    if value.ends_with('\n') {
        output.push(String::new());
    }
    output.join("\n")
}

fn redact_line(
    value: &str,
    mut redact_next: bool,
    mut redact_authorization_scheme: bool,
) -> (String, bool, bool, bool) {
    let mut output = Vec::new();
    let mut redact_continuation = false;

    let mut tokens = value.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        if redact_next {
            if is_bearer_word(token) || (redact_authorization_scheme && is_auth_scheme_word(token))
            {
                output.push("<redacted>".to_owned());
                continue;
            }
            if is_separator(token) {
                output.push(token.to_owned());
                continue;
            }
            if is_redaction_connector(token) {
                output.push(token.to_owned());
                continue;
            }
            output.push("<redacted>".to_owned());
            redact_next = false;
            redact_authorization_scheme = false;
            continue;
        }

        if let Some(has_value) = sensitive_assignment(token) {
            output.push("<redacted>".to_owned());
            redact_next = !has_value || has_embedded_authorization_scheme(token);
            redact_authorization_scheme = is_authorization_key(token) && redact_next;
            redact_continuation = true;
        } else if is_sensitive_key(token)
            || (is_bare_sensitive_label(token)
                && tokens
                    .peek()
                    .is_some_and(|next| is_redaction_connector(next)))
        {
            output.push("<redacted>".to_owned());
            redact_next = true;
            redact_authorization_scheme = is_authorization_key(token);
        } else if is_bearer_word(token) {
            output.push("<redacted>".to_owned());
            redact_next = true;
        } else if is_secret_token(token) {
            output.push("<redacted>".to_owned());
        } else {
            output.push(token.to_owned());
        }
    }

    (
        output.join(" "),
        redact_next,
        redact_authorization_scheme,
        redact_continuation,
    )
}

fn leading_whitespace(value: &str) -> usize {
    value
        .chars()
        .take_while(|character| character.is_whitespace())
        .map(|character| if character == '\t' { 4 } else { 1 })
        .sum()
}

fn is_private_key_begin(line: &str) -> bool {
    let line = line.to_ascii_uppercase();
    line.contains("-----BEGIN ") && line.contains("PRIVATE KEY")
}

fn is_private_key_end(line: &str) -> bool {
    let line = line.to_ascii_uppercase();
    line.contains("-----END ") && line.contains("PRIVATE KEY")
}

pub fn redact_path(path: &Path) -> String {
    let mut redacted = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => redacted.push(prefix.as_os_str()),
            Component::RootDir => redacted.push(component.as_os_str()),
            Component::CurDir => redacted.push("."),
            Component::ParentDir => redacted.push(".."),
            Component::Normal(value) => redacted.push(redact(&value.to_string_lossy())),
        }
    }
    redacted.display().to_string()
}

fn sensitive_assignment(token: &str) -> Option<bool> {
    let token = trim_wrappers(token);
    for (index, character) in token.char_indices() {
        if !matches!(character, ':' | '=') {
            continue;
        }
        let key = trim_wrappers(&token[..index]);
        if !is_sensitive_assignment_key(key) {
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
        if !is_authorization_key(token) {
            continue;
        }
        let value = trim_wrappers(&token[index + character.len_utf8()..]);
        return value.eq_ignore_ascii_case("bearer") || value.eq_ignore_ascii_case("basic");
    }
    false
}

fn is_sensitive_key(token: &str) -> bool {
    let key = trim_wrappers(token).trim_matches(|character| matches!(character, ':' | '='));
    !key.eq_ignore_ascii_case("token") && is_sensitive_identifier(&key.to_ascii_lowercase())
}

fn is_sensitive_assignment_key(token: &str) -> bool {
    let key = trim_wrappers(token).trim_matches(|character| matches!(character, ':' | '='));
    let normalized = normalize_identifier(key);
    normalized == "key" || is_sensitive_identifier(&normalized)
}

fn normalize_identifier(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn is_sensitive_identifier(normalized: &str) -> bool {
    SENSITIVE_KEYS.contains(&normalized)
        || normalized
            .split('_')
            .any(|part| SENSITIVE_KEY_PARTS.contains(&part))
        || normalized.ends_with("_key")
        || normalized.ends_with("_token")
        || normalized
            .split('_')
            .collect::<Vec<_>>()
            .windows(2)
            .any(|parts| {
                matches!(
                    parts,
                    ["api", "key"] | ["access", "key"] | ["private", "key"]
                )
            })
}

fn is_auth_scheme_word(token: &str) -> bool {
    let token = trim_wrappers(token).trim_matches(|character| matches!(character, ':' | '='));
    token.eq_ignore_ascii_case("bearer") || token.eq_ignore_ascii_case("basic")
}

fn is_bearer_word(token: &str) -> bool {
    let token = trim_wrappers(token).trim_matches(|character| matches!(character, ':' | '='));
    token.eq_ignore_ascii_case("bearer")
}

fn is_authorization_key(token: &str) -> bool {
    let token = trim_wrappers(token);
    let key = token
        .char_indices()
        .find(|(_, character)| matches!(character, ':' | '='))
        .map(|(index, _)| &token[..index])
        .unwrap_or(token);
    normalize_identifier(
        trim_wrappers(key).trim_matches(|character| matches!(character, ':' | '=')),
    )
    .split('_')
    .any(|part| part == "authorization")
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

fn is_bare_sensitive_label(token: &str) -> bool {
    let label = trim_wrappers(token)
        .trim_matches(|character| matches!(character, ':' | '='))
        .to_ascii_lowercase();
    matches!(label.as_str(), "key" | "token")
}

fn is_redaction_connector(token: &str) -> bool {
    if is_separator(token) {
        return true;
    }
    let token = trim_wrappers(token).trim_matches(|character| matches!(character, ':' | '='));
    token.eq_ignore_ascii_case("is") || token.eq_ignore_ascii_case("was")
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

#[cfg(test)]
mod tests {
    use super::redact_path;
    use std::path::Path;

    #[test]
    fn redacted_path_preserves_parent_components() {
        assert!(redact_path(Path::new("private/../secret")).contains(".."));
    }
}
