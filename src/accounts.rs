//! Cloud-provider API keys: macOS Keychain first, then environment variables.
//! Key values are never logged.

use crate::types::CloudProvider;
use anyhow::{anyhow, Result};
use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const SERVICE: &str = "dev.naeem.murmur";
/// errSecItemNotFound
const ERR_NOT_FOUND: i32 = -25300;

fn account(p: CloudProvider) -> &'static str {
    match p {
        CloudProvider::Claude => "anthropic-api-key",
        CloudProvider::ChatGpt => "openai-api-key",
        CloudProvider::Grok => "xai-api-key",
    }
}

fn env_name(p: CloudProvider) -> &'static str {
    match p {
        CloudProvider::Claude => "ANTHROPIC_API_KEY",
        CloudProvider::ChatGpt => "OPENAI_API_KEY",
        CloudProvider::Grok => "XAI_API_KEY",
    }
}

pub fn console_url(p: CloudProvider) -> &'static str {
    match p {
        CloudProvider::Claude => "https://platform.claude.com/",
        CloudProvider::ChatGpt => "https://platform.openai.com/api-keys",
        CloudProvider::Grok => "https://console.x.ai",
    }
}

static KEYCHAIN_CACHE: Mutex<Option<HashMap<CloudProvider, Option<String>>>> = Mutex::new(None);

fn keychain_key(p: CloudProvider) -> Option<String> {
    let mut guard = KEYCHAIN_CACHE.lock().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(v) = cache.get(&p) {
        return v.clone();
    }
    let v = match get_generic_password(SERVICE, account(p)) {
        Ok(bytes) => String::from_utf8(bytes).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()),
        Err(e) => {
            if e.code() != ERR_NOT_FOUND {
                log::warn!("keychain read for {} failed: code {}", p.label(), e.code());
            }
            None
        }
    };
    cache.insert(p, v.clone());
    v
}

fn invalidate(p: CloudProvider) {
    if let Some(c) = KEYCHAIN_CACHE.lock().unwrap().as_mut() {
        c.remove(&p);
    }
}

/// Environment lookup, cached for the process lifetime. Apps launched from Finder don't inherit
/// shell variables, so Claude's key is also read from launchd.
fn env_key(p: CloudProvider) -> Option<&'static str> {
    static CLAUDE: OnceLock<Option<String>> = OnceLock::new();
    static OPENAI: OnceLock<Option<String>> = OnceLock::new();
    static XAI: OnceLock<Option<String>> = OnceLock::new();
    let cell = match p {
        CloudProvider::Claude => &CLAUDE,
        CloudProvider::ChatGpt => &OPENAI,
        CloudProvider::Grok => &XAI,
    };
    cell.get_or_init(|| {
        let name = env_name(p);
        let from_env = std::env::var(name).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        if p != CloudProvider::Claude {
            return from_env;
        }
        from_env.or_else(|| {
            let out = std::process::Command::new("launchctl").args(["getenv", name]).output().ok()?;
            Some(String::from_utf8(out.stdout).ok()?.trim().to_string()).filter(|k| !k.is_empty())
        })
    })
    .as_deref()
}

pub fn api_key(p: CloudProvider) -> Option<String> {
    keychain_key(p).or_else(|| env_key(p).map(str::to_string))
}

/// Where `api_key` would read from, for the UI.
pub fn key_source(p: CloudProvider) -> Option<&'static str> {
    if keychain_key(p).is_some() {
        Some("Keychain")
    } else if env_key(p).is_some() {
        Some("environment")
    } else {
        None
    }
}

pub fn set_api_key(p: CloudProvider, key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        return Err(anyhow!("API key is empty"));
    }
    let r = set_generic_password(SERVICE, account(p), key.as_bytes());
    invalidate(p);
    r.map_err(|e| anyhow!("could not save to Keychain (code {})", e.code()))
}

/// Removes the Keychain entry; a missing entry is not an error.
pub fn delete_api_key(p: CloudProvider) -> Result<()> {
    let r = delete_generic_password(SERVICE, account(p));
    invalidate(p);
    match r {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERR_NOT_FOUND => Ok(()),
        Err(e) => Err(anyhow!("could not remove from Keychain (code {})", e.code())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_urls() {
        assert_eq!(console_url(CloudProvider::Claude), "https://platform.claude.com/");
        assert_eq!(console_url(CloudProvider::ChatGpt), "https://platform.openai.com/api-keys");
        assert_eq!(console_url(CloudProvider::Grok), "https://console.x.ai");
    }

    #[test]
    fn accounts_are_distinct() {
        let a: std::collections::HashSet<_> = CloudProvider::ALL.iter().map(|p| account(*p)).collect();
        assert_eq!(a.len(), 3);
    }

    #[test]
    fn empty_key_rejected() {
        assert!(set_api_key(CloudProvider::Claude, "   ").is_err());
    }
}
