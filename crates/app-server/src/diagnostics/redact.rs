use regex::Regex;
use serde_json::Value;
use std::{path::Path, sync::LazyLock};
static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)((?:authkey|access_token|refresh_token|authorization|cookie|session_key|encryption_key|client_secret|seed|token|password|secret)\s*[=:]\s*)[^&\s,;\}"]+"#).unwrap()
});
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._~+/=-]+").unwrap());
static USER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)[A-Z]:[\\/]Users[\\/][^\\/\s"']+"#).unwrap());
pub struct Cleaner {
    root: String,
    home: Option<String>,
}
impl Cleaner {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_string_lossy().into_owned(),
            home: std::env::var("USERPROFILE").ok(),
        }
    }
    pub fn text(&self, s: &str) -> String {
        let mut s = s.replace(&self.root, "<local-data>");
        if let Some(home) = &self.home {
            s = s.replace(home, "<user>");
        }
        let s = USER.replace_all(&s, "<user>");
        let s = BEARER.replace_all(&s, "Bearer [REDACTED]");
        SECRET.replace_all(&s, "${1}[REDACTED]").into_owned()
    }
    pub fn value(&self, v: &mut Value) {
        match v {
            Value::Object(map) => {
                for (key, value) in map {
                    let k = key.to_ascii_lowercase().replace('-', "_");
                    if [
                        "key",
                        "keys",
                        "seed",
                        "token",
                        "authkey",
                        "authorization",
                        "cookie",
                        "cookies",
                        "password",
                        "secret",
                        "client_secret",
                        "encryption_key",
                        "encryption_key_seed",
                        "session_key",
                        "deduced_keys",
                        "access_token",
                        "refresh_token",
                    ]
                    .contains(&k.as_str())
                        || k.ends_with("_token")
                        || k.ends_with("_secret")
                        || k.ends_with("_seed")
                        || (k.ends_with("_key") && k != "entity_key")
                    {
                        *value = Value::String("[REDACTED]".into());
                    } else {
                        self.value(value);
                    }
                }
            }
            Value::Array(rows) => {
                for row in rows {
                    self.value(row);
                }
            }
            Value::String(s) => *s = self.text(s),
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn removes_secrets_without_destroying_inventory_keys() {
        let clean = Cleaner::new(Path::new("C:/test/local-data"));
        let mut v = json!({"authkey":"private-key","nested":{"session_key":"private-session","entity_key":"123"},"text":"https://host/?authkey=private-url&uid=711 Bearer private-bearer C:/test/local-data/run"});
        clean.value(&mut v);
        let text = v.to_string();
        assert!(!text.contains("private-"));
        assert!(text.contains("entity_key"));
        assert!(text.contains("<local-data>"));
        assert!(text.contains("711"));
    }
}
