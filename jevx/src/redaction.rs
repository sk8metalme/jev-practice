use sha2::{Digest, Sha256};

pub fn redact(value: &str) -> String {
    value
        .split_whitespace()
        .map(|token| {
            let lower = token.to_ascii_lowercase();
            let looks_sensitive = lower.contains("token=")
                || lower.contains("api_key=")
                || lower.contains("secret=")
                || lower.contains("password=")
                || lower.starts_with("bearer")
                || lower.starts_with("sk-")
                || lower.starts_with("tsk-");
            if looks_sensitive {
                "<redacted>".to_owned()
            } else {
                token.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}
