//! Sign in with ChatGPT: OpenAI's "ChatGPT plan usage in your open-source app" flow.
//!
//! Follows https://developers.openai.com/siwc/token-sharing-open-source (and its sign-in,
//! profiles-and-sessions, token-reference and errors-and-recovery pages): dynamic client
//! registration with `client_id=dynamic_agent_client`, a loopback redirect on 127.0.0.1, PKCE S256,
//! rotating refresh tokens. Endpoints match https://auth.openai.com/.well-known/openid-configuration.
//!
//! Keychain (service "dev.naeem.murmur"):
//! - "chatgpt-host-id": this Mac's stable `ext_agent_host_id` (`urn:uuid:<v4>`), kept across sign-outs.
//! - "chatgpt-oauth": the signed-in session (tokens, issued client id, scopes, expiry, identity).
//! - "chatgpt-registration": the issued client id and identity, kept after sign-out so signing in
//!   again reuses the registration instead of creating a new client (profiles-and-sessions.md).
//!
//! Tokens are never logged, and authorization URLs (which may carry `id_token_hint`) are not logged.

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Where users review or change how much of their plan Murmur may use (ChatGPT Settings → Usage).
pub const MANAGE_USAGE_URL: &str = "https://chatgpt.com/settings/usage";

#[derive(Debug, Clone)]
pub struct Account {
    pub email: Option<String>,
    /// The grant includes `chatgpt.tokens.use.direct`, so cleanup can run on the user's plan.
    pub plan_usage: bool,
}

const SERVICE: &str = "dev.naeem.murmur";
const SESSION_ACCOUNT: &str = "chatgpt-oauth";
const HOST_ID_ACCOUNT: &str = "chatgpt-host-id";
const REGISTRATION_ACCOUNT: &str = "chatgpt-registration";
/// errSecItemNotFound
const ERR_NOT_FOUND: i32 = -25300;

const ISSUER: &str = "https://auth.openai.com";
const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const REVOKE_URL: &str = "https://auth.openai.com/api/accounts/oauth/revoke";
const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
const AGENT_NAME: &str = "Murmur";
const CALLBACK_PATH: &str = "/auth/callback";
const PREFERRED_PORT: u16 = 1455;
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Refresh when the access token has less than this long left.
const REFRESH_MARGIN_SECS: u64 = 60;
/// Tolerated clock skew when checking the ID token's `exp`.
const CLOCK_SKEW_SECS: u64 = 120;

// ---------------------------------------------------------------------------------------------
// Stored state
// ---------------------------------------------------------------------------------------------

/// No Debug derive: it holds tokens.
#[derive(Clone, Serialize, Deserialize)]
struct Session {
    ext_agent_host_id: String,
    /// The issued client id (`oaiapp_...`), never `dynamic_agent_client`.
    client_id: String,
    access_token: String,
    refresh_token: String,
    /// Access-token expiry, unix seconds.
    expires_at: u64,
    scopes: Vec<String>,
    email: Option<String>,
    /// Validated ID-token `sub`.
    subject: String,
    /// Retained for `id_token_hint` on reauthorization.
    id_token: String,
}

impl Session {
    fn plan_usage(&self) -> bool {
        self.scopes.iter().any(|s| s == PLAN_SCOPE)
    }
}

/// What survives sign-out: the registration to reuse on the next sign-in.
#[derive(Clone, Serialize, Deserialize)]
struct Registration {
    client_id: String,
    /// Validated ID-token `sub`. None when the issued client id was saved before any code exchange
    /// succeeded (the sign-in.md §3 `invalid_grant` recovery); the next successful sign-in fills it.
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    email: Option<String>,
    /// Dropped on sign-out (the docs say to omit `id_token_hint` after signing out).
    #[serde(default)]
    id_token: Option<String>,
}

/// Outer None: not loaded from the Keychain yet.
static SESSION: Mutex<Option<Option<Session>>> = Mutex::new(None);
/// Serializes refreshes (the refresh token rotates), sign-in completion and sign-out.
static REFRESH: Mutex<()> = Mutex::new(());

fn kc_get(account: &str) -> Option<String> {
    match get_generic_password(SERVICE, account) {
        Ok(bytes) => String::from_utf8(bytes).ok().filter(|s| !s.trim().is_empty()),
        Err(e) => {
            if e.code() != ERR_NOT_FOUND {
                log::warn!("keychain read for {account} failed: code {}", e.code());
            }
            None
        }
    }
}

fn kc_set(account: &str, value: &str) -> Result<()> {
    set_generic_password(SERVICE, account, value.as_bytes())
        .map_err(|e| anyhow!("could not save to Keychain (code {})", e.code()))
}

fn kc_delete(account: &str) -> Result<()> {
    match delete_generic_password(SERVICE, account) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERR_NOT_FOUND => Ok(()),
        Err(e) => Err(anyhow!("could not remove from Keychain (code {})", e.code())),
    }
}

fn load_json<T: for<'de> Deserialize<'de>>(account: &str) -> Option<T> {
    let raw = kc_get(account)?;
    match serde_json::from_str(&raw) {
        Ok(v) => Some(v),
        Err(_) => {
            log::warn!("ignoring unreadable Keychain item {account}");
            None
        }
    }
}

fn cached_session() -> Option<Session> {
    let mut guard = SESSION.lock().unwrap();
    if guard.is_none() {
        *guard = Some(load_json::<Session>(SESSION_ACCOUNT));
    }
    guard.clone().flatten()
}

/// Updates the cache first so a Keychain failure can't resurrect a rotated-away refresh token.
fn save_session(s: &Session) -> Result<()> {
    *SESSION.lock().unwrap() = Some(Some(s.clone()));
    kc_set(SESSION_ACCOUNT, &serde_json::to_string(s)?)
}

fn clear_session() -> Result<()> {
    *SESSION.lock().unwrap() = Some(None);
    kc_delete(SESSION_ACCOUNT)
}

fn load_registration() -> Option<Registration> {
    load_json(REGISTRATION_ACCOUNT)
}

fn save_registration(r: &Registration) -> Result<()> {
    kc_set(REGISTRATION_ACCOUNT, &serde_json::to_string(r)?)
}

/// This host's stable `ext_agent_host_id`, created once (overview page: "Host-ID").
fn host_id() -> Result<String> {
    if let Some(id) = kc_get(HOST_ID_ACCOUNT) {
        return Ok(id.trim().to_string());
    }
    let id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    kc_set(HOST_ID_ACCOUNT, &id).context("saving the ChatGPT host id")?;
    Ok(id)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn auth_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into()
}

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

/// Signed-in account from the Keychain, without network access.
pub fn account() -> Option<Account> {
    cached_session().map(|s| Account { email: s.email.clone(), plan_usage: s.plan_usage() })
}

/// Opens the system browser and blocks until the user finishes (or 5 minutes pass).
pub fn sign_in() -> Result<Account> {
    let host = host_id()?;
    let registration = load_registration();

    // Listen before opening the browser (sign-in.md §2). Only the port may vary between sign-ins.
    let listener = TcpListener::bind(("127.0.0.1", PREFERRED_PORT))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .context("could not start the local sign-in listener")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = redirect_uri(port);

    log::info!(
        "ChatGPT sign-in: waiting on port {port} ({})",
        if registration.is_some() { "saved registration" } else { "new registration" }
    );
    let mut flow = Flow::new(host, redirect_uri, registration, Secrets::random()?);
    std::process::Command::new("open").arg(flow.authorize_url()).status().context("could not open the browser")?;

    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            bail!("ChatGPT sign-in timed out after 5 minutes");
        }
        let mut stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e).context("sign-in listener failed"),
        };
        // Accepted sockets inherit O_NONBLOCK on macOS.
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let Some(target) = read_request_target(&mut stream) else {
            respond(&mut stream, "400 Bad Request", &page("Bad request", "Murmur couldn't read that request."));
            continue;
        };
        let (path, query) = split_target(&target);
        if path != CALLBACK_PATH {
            respond(&mut stream, "404 Not Found", &page("Not found", "Nothing here."));
            continue;
        }
        match flow.on_callback(query) {
            CallbackAction::Ignore { status, title, message } => {
                respond(&mut stream, status, &page(title, message));
            }
            CallbackAction::Fail { code, message } => {
                respond(&mut stream, "200 OK", &page("Sign-in didn't finish", &message));
                if flow.attempt.registration.is_some()
                    && matches!(code.as_deref(), Some("invalid_client" | "unauthorized_client"))
                {
                    // The saved registration is gone (e.g. disconnected in ChatGPT): register afresh next time.
                    let _ = kc_delete(REGISTRATION_ACCOUNT);
                }
                bail!(message);
            }
            CallbackAction::Exchange(ex) => match exchange_code(&flow, &ex) {
                Ok(account) => {
                    respond(&mut stream, "200 OK", &page("Signed in", "You're signed in to Murmur — you can close this tab."));
                    return Ok(account);
                }
                Err(SignInError::Unusable(err)) => match flow.on_unusable(&ex, &err, Secrets::random()?) {
                    RetryDecision::Retry { location, registration } => {
                        // sign-in.md §3: keep the issued client id and authorize afresh with it.
                        if let Err(e) = save_registration(&registration) {
                            log::warn!("ChatGPT sign-in: could not save the issued client id: {e:#}");
                        }
                        log::info!("ChatGPT sign-in: code rejected ({}); retrying once with the issued client id", err.code);
                        respond_redirect(&mut stream, &location);
                    }
                    RetryDecision::GiveUp(message) => {
                        respond(&mut stream, "200 OK", &page("Sign-in didn't finish", &message));
                        bail!(message);
                    }
                },
                Err(SignInError::Fatal(e)) => {
                    respond(&mut stream, "200 OK", &page("Sign-in didn't finish", &format!("{e:#}")));
                    return Err(e);
                }
            },
        }
    }
}

/// Revokes the refresh token and forgets the session.
pub fn sign_out() -> Result<()> {
    let _guard = REFRESH.lock().unwrap();
    let session = cached_session();
    let revoked = match &session {
        Some(s) => revoke(&s.client_id, &s.refresh_token),
        None => true,
    };
    let cleared = clear_session();
    // Keep the registration for the next sign-in, minus the ID token hint.
    if let Some(mut r) = load_registration() {
        r.id_token = None;
        if let Err(e) = save_registration(&r) {
            log::warn!("ChatGPT sign-out: could not update registration: {e:#}");
        }
    }
    cleared?;
    log::info!("ChatGPT sign-out: done (revocation {})", if revoked { "confirmed" } else { "not confirmed" });
    if !revoked {
        bail!(
            "Signed out of ChatGPT in Murmur, but OpenAI didn't confirm ending the session. \
             You can disconnect Murmur in ChatGPT settings."
        );
    }
    Ok(())
}

/// A valid access token with plan usage, refreshing it when needed; None when signed out.
pub fn access_token() -> Option<String> {
    let s = cached_session()?;
    if !s.plan_usage() {
        return None;
    }
    if !needs_refresh(s.expires_at, now_secs()) {
        return Some(s.access_token);
    }
    let _guard = REFRESH.lock().unwrap();
    // Another thread may have refreshed (or signed out) while we waited.
    let s = cached_session()?;
    if !needs_refresh(s.expires_at, now_secs()) {
        return s.plan_usage().then_some(s.access_token);
    }
    match refresh(&s) {
        Ok(new) => {
            if let Err(e) = save_session(&new) {
                log::warn!("ChatGPT: refreshed session not saved to Keychain: {e:#}");
            }
            log::info!("ChatGPT: access token refreshed");
            new.plan_usage().then_some(new.access_token)
        }
        Err(TokenError::Unusable(err)) => {
            log::warn!("ChatGPT: session no longer valid ({}); signed out", err.code);
            let _ = clear_session();
            None
        }
        Err(TokenError::InvalidClient(_)) => {
            log::warn!("ChatGPT: registration rejected (invalid_client); signed out");
            let _ = clear_session();
            let _ = kc_delete(REGISTRATION_ACCOUNT);
            None
        }
        Err(TokenError::Other(e)) => {
            // Temporary failure: keep the credentials (errors-and-recovery.md, "Disconnection").
            log::warn!("ChatGPT: token refresh failed: {e}");
            (s.expires_at > now_secs()).then_some(s.access_token)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Network steps
// ---------------------------------------------------------------------------------------------

enum SignInError {
    /// The token endpoint said the code can't be used (`invalid_grant` etc.).
    Unusable(OAuthError),
    Fatal(anyhow::Error),
}

/// Exchanges the current attempt's code (sign-in.md §3 step 3) and saves the result.
fn exchange_code(flow: &Flow, ex: &Exchange) -> Result<Account, SignInError> {
    let a = &flow.attempt;
    let tokens = token_request(&exchange_form(ex, &a.verifier, &flow.redirect_uri)).map_err(|e| match e {
        TokenError::Unusable(err) => SignInError::Unusable(err),
        TokenError::InvalidClient(err) => {
            if a.registration.is_some() {
                let _ = kc_delete(REGISTRATION_ACCOUNT);
            }
            SignInError::Fatal(anyhow!("OpenAI rejected Murmur's registration ({err}). Please try signing in again."))
        }
        TokenError::Other(m) => SignInError::Fatal(anyhow!("ChatGPT token exchange failed: {m}")),
    })?;
    finish_sign_in(flow, ex, tokens).map_err(SignInError::Fatal)
}

/// The `authorization_code` grant: issued client id, the exact verifier and redirect URI of the
/// attempt that produced the code, and the same resource as the authorization request.
fn exchange_form<'a>(ex: &'a Exchange, verifier: &'a str, redirect_uri: &'a str) -> [(&'static str, &'a str); 6] {
    [
        ("grant_type", "authorization_code"),
        ("client_id", &ex.client_id),
        ("code", &ex.code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect_uri),
        ("resource", RESOURCE),
    ]
}

fn finish_sign_in(flow: &Flow, ex: &Exchange, tokens: TokenSet) -> Result<Account> {
    let a = &flow.attempt;
    let client_id = ex.client_id.clone();
    let id_token = tokens.id_token.ok_or_else(|| anyhow!("ChatGPT didn't return an ID token"))?;
    let claims = validate_id_token(&id_token, &client_id, &a.nonce, now_secs())?;
    if let Some(saved) = a.registration.as_ref().and_then(|r| r.subject.as_deref()) {
        if saved != claims.sub {
            // Keep accounts apart: the next attempt registers the newly chosen account from scratch.
            let _ = kc_delete(REGISTRATION_ACCOUNT);
            bail!("That's a different ChatGPT account from the one Murmur was registered with. Sign in again to add it.");
        }
    }
    let refresh_token = tokens.refresh_token.ok_or_else(|| anyhow!("ChatGPT didn't return a refresh token"))?;
    let scopes = parse_scopes(tokens.scope.as_deref().or(ex.cb_scope.as_deref()).unwrap_or(""));
    let session = Session {
        ext_agent_host_id: flow.host.clone(),
        client_id: client_id.clone(),
        access_token: tokens.access_token,
        refresh_token,
        expires_at: now_secs() + tokens.expires_in,
        scopes,
        email: claims.email.clone(),
        subject: claims.sub.clone(),
        id_token: id_token.clone(),
    };
    {
        let _guard = REFRESH.lock().unwrap();
        save_session(&session)?;
    }
    save_registration(&Registration {
        client_id,
        subject: Some(claims.sub),
        email: claims.email.clone(),
        id_token: Some(id_token),
    })?;
    let plan_usage = session.plan_usage();
    log::info!("ChatGPT sign-in: complete (plan usage {})", if plan_usage { "granted" } else { "not granted" });
    Ok(Account { email: claims.email, plan_usage })
}

fn refresh(s: &Session) -> Result<Session, TokenError> {
    // No `scope`: the grant is retained (profiles-and-sessions.md, "Refreshing tokens").
    let t = token_request(&[
        ("grant_type", "refresh_token"),
        ("client_id", &s.client_id),
        ("refresh_token", &s.refresh_token),
        ("resource", RESOURCE),
    ])?;
    let mut new = s.clone();
    new.access_token = t.access_token;
    if let Some(rt) = t.refresh_token {
        new.refresh_token = rt;
    }
    new.expires_at = now_secs() + t.expires_in;
    if let Some(scope) = t.scope {
        new.scopes = parse_scopes(&scope);
    }
    Ok(new)
}

fn token_request(form: &[(&str, &str)]) -> Result<TokenSet, TokenError> {
    let mut resp = auth_agent()
        .post(TOKEN_URL)
        .header("Accept", "application/json")
        .send_form(form.iter().copied())
        .map_err(|e| TokenError::Other(format!("network error: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    let result = parse_token_response(status, &body);
    if let Err(e) = &result {
        // Only the status and OAuth error fields: never the code, verifier, tokens or URLs.
        let grant = form.iter().find(|(k, _)| *k == "grant_type").map_or("?", |(_, v)| *v);
        log::warn!("ChatGPT token endpoint ({grant}) failed: {}", e.log_line());
    }
    result
}

/// Best effort with one retry; true when OpenAI confirmed (empty 200, even for an already-invalid token).
fn revoke(client_id: &str, refresh_token: &str) -> bool {
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(1));
        }
        let r = auth_agent().post(REVOKE_URL).send_form([
            ("token", refresh_token),
            ("token_type_hint", "refresh_token"),
            ("client_id", client_id),
        ]);
        match r {
            Ok(resp) if resp.status().is_success() => return true,
            Ok(resp) if resp.status().is_client_error() => {
                log::warn!("ChatGPT revoke: HTTP {}", resp.status());
                return false;
            }
            Ok(resp) => log::warn!("ChatGPT revoke: HTTP {}", resp.status()),
            Err(e) => log::warn!("ChatGPT revoke: {e}"),
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------------------------

fn random_b64(n: usize) -> Result<String> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|e| anyhow!("no secure randomness: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// RFC 7636 S256: base64url(sha256(verifier)) without padding.
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{CALLBACK_PATH}")
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes
                    .get(i + 1..i + 3)
                    .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
                    .and_then(|h| std::str::from_utf8(h).ok());
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

struct AuthorizeParams<'a> {
    /// The saved issued client id on reauthorization; None registers a new client.
    client_id: Option<&'a str>,
    id_token_hint: Option<&'a str>,
    login_hint: Option<&'a str>,
    host_id: &'a str,
    redirect_uri: &'a str,
    state: &'a str,
    nonce: &'a str,
    code_challenge: &'a str,
}

/// Authorization request per sign-in.md §2.
fn authorize_url(p: &AuthorizeParams) -> String {
    let mut q: Vec<(&str, &str)> = Vec::new();
    match p.client_id {
        Some(id) => {
            q.push(("client_id", id));
            if let Some(h) = p.id_token_hint {
                q.push(("id_token_hint", h));
            }
            if let Some(h) = p.login_hint {
                q.push(("login_hint", h));
            }
        }
        None => {
            q.push(("client_id", DYNAMIC_CLIENT));
            q.push(("agent_name_hint", AGENT_NAME));
        }
    }
    q.extend([
        ("ext_agent_host_id", p.host_id),
        ("response_type", "code"),
        ("redirect_uri", p.redirect_uri),
        ("scope", SCOPES),
        ("resource", RESOURCE),
        ("state", p.state),
        ("nonce", p.nonce),
        ("code_challenge", p.code_challenge),
        ("code_challenge_method", "S256"),
    ]);
    let qs: Vec<String> = q.iter().map(|(k, v)| format!("{k}={}", percent_encode(v))).collect();
    format!("{AUTHORIZE_URL}?{}", qs.join("&"))
}

#[derive(Debug, PartialEq)]
enum CallbackOutcome {
    Code { code: String, client_id: Option<String>, scope: Option<String> },
    Error { code: String, description: Option<String> },
    /// Missing or different `state`: not our callback.
    BadState,
}

/// Validates `state` before anything else, then handles `error` (sign-in.md §3).
fn parse_callback(query: &str, expected_state: &str) -> CallbackOutcome {
    let params = parse_query(query);
    let get = |k: &str| params.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
    if get("state").as_deref() != Some(expected_state) {
        return CallbackOutcome::BadState;
    }
    if let Some(code) = get("error") {
        return CallbackOutcome::Error { code, description: get("error_description") };
    }
    match get("code") {
        Some(code) => CallbackOutcome::Code { code, client_id: get("client_id"), scope: get("scope") },
        None => CallbackOutcome::Error { code: "invalid_response".into(), description: Some("no authorization code".into()) },
    }
}

fn callback_error_message(code: &str, description: Option<&str>) -> String {
    if code == "access_denied" {
        return "Sign-in was cancelled. You can try again any time.".into();
    }
    let d: String = description.unwrap_or("").chars().take(200).collect();
    if d.is_empty() {
        format!("ChatGPT sign-in failed ({code}).")
    } else {
        format!("ChatGPT sign-in failed ({code}): {d}")
    }
}

/// The client id to exchange the code with: the saved one on reauthorization (the callback may
/// omit it but must not change it), else the issued one from the new-registration callback.
fn resolve_client_id(pending: Option<&str>, from_callback: Option<&str>) -> Result<String> {
    match (pending, from_callback) {
        (Some(saved), None) => Ok(saved.to_string()),
        (Some(saved), Some(cb)) if cb == saved => Ok(saved.to_string()),
        (Some(_), Some(_)) => bail!("ChatGPT returned a different client id than Murmur's registration; sign-in rejected"),
        (None, Some(cb)) if cb != DYNAMIC_CLIENT => Ok(cb.to_string()),
        (None, _) => bail!("ChatGPT didn't return Murmur's issued client id; registration is incomplete. Please try again."),
    }
}

/// Request target ("/auth/callback?code=...") from an HTTP request line, reading at most 16 KB.
fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    request_target(&String::from_utf8_lossy(&buf)).map(str::to_string)
}

fn request_target(req: &str) -> Option<&str> {
    let line = req.lines().next()?;
    let mut parts = line.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;
    (method == "GET" && target.starts_with('/')).then_some(target)
}

fn split_target(target: &str) -> (&str, &str) {
    target.split_once('?').unwrap_or((target, ""))
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn page(title: &str, message: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\">\
         <title>Murmur – {t}</title><style>body{{font:16px -apple-system,system-ui,sans-serif;margin:0;\
         min-height:100vh;display:flex;align-items:center;justify-content:center;background:#f5f5f7;color:#1d1d1f}}\
         @media(prefers-color-scheme:dark){{body{{background:#1d1d1f;color:#f5f5f7}}}}\
         main{{max-width:28rem;padding:2rem;text-align:center}}h1{{font-size:1.4rem}}</style></head>\
         <body><main><h1>{t}</h1><p>{m}</p></main></body></html>",
        t = html_escape(title),
        m = html_escape(message)
    )
}

#[derive(Debug, PartialEq)]
struct IdClaims {
    sub: String,
    email: Option<String>,
}

/// Checks the ID token's iss, aud, exp and nonce from its payload.
///
/// The signature is not verified against the JWKS: the token comes straight from OpenAI's token
/// endpoint over TLS in the code exchange, and OpenID Connect Core §3.1.3.7 (6) allows TLS server
/// validation in place of the signature check for tokens received that way.
fn validate_id_token(jwt: &str, client_id: &str, nonce: &str, now: u64) -> Result<IdClaims> {
    let payload = jwt.split('.').nth(1).ok_or_else(|| anyhow!("malformed ID token"))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| anyhow!("malformed ID token payload"))?;
    let c: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| anyhow!("malformed ID token claims"))?;
    anyhow::ensure!(c["iss"].as_str() == Some(ISSUER), "ID token has the wrong issuer");
    let aud_ok = match &c["aud"] {
        serde_json::Value::String(a) => a == client_id,
        serde_json::Value::Array(a) => a.iter().any(|v| v.as_str() == Some(client_id)),
        _ => false,
    };
    anyhow::ensure!(aud_ok, "ID token is for a different client");
    let exp = c["exp"].as_u64().ok_or_else(|| anyhow!("ID token has no expiry"))?;
    anyhow::ensure!(exp + CLOCK_SKEW_SECS > now, "ID token has expired");
    anyhow::ensure!(c["nonce"].as_str() == Some(nonce), "ID token nonce doesn't match this sign-in");
    let sub = c["sub"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("ID token has no subject"))?;
    Ok(IdClaims { sub: sub.to_string(), email: c["email"].as_str().map(str::to_string) })
}

fn parse_scopes(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

fn needs_refresh(expires_at: u64, now: u64) -> bool {
    expires_at <= now + REFRESH_MARGIN_SECS
}

struct TokenSet {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: u64,
    scope: Option<String>,
}

/// An OAuth error from the token endpoint. None of these fields is secret, so they may be logged.
#[derive(Debug, Clone, PartialEq)]
struct OAuthError {
    status: u16,
    /// The OAuth `error` code, e.g. `invalid_grant` (`http_<status>` when the body had none).
    code: String,
    description: Option<String>,
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.description {
            Some(d) => write!(f, "{}: {d}", self.code),
            None => f.write_str(&self.code),
        }
    }
}

#[derive(Debug, PartialEq)]
enum TokenError {
    /// The grant or refresh token can't be used again: sign in again.
    Unusable(OAuthError),
    InvalidClient(OAuthError),
    /// Network, server or unexpected errors; credentials are kept.
    Other(String),
}

impl TokenError {
    fn log_line(&self) -> String {
        match self {
            TokenError::Unusable(e) | TokenError::InvalidClient(e) => format!(
                "HTTP {}, error={}, error_description={}",
                e.status,
                e.code,
                e.description.as_deref().unwrap_or("(none)")
            ),
            TokenError::Other(m) => m.clone(),
        }
    }
}

/// What the user sees when the code exchange is refused, with OpenAI's error code.
fn unusable_message(e: &OAuthError) -> String {
    format!("OpenAI rejected the sign-in code ({e}). Please try signing in again.")
}

/// Bounded, single-line diagnostic text from the server.
fn clean_description(s: &str) -> Option<String> {
    let d: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(200).collect();
    let d = d.trim();
    (!d.is_empty()).then(|| d.to_string())
}

/// Codes that mean the refresh token is unusable (errors-and-recovery.md, "Refresh errors").
const UNUSABLE_CODES: &[&str] = &[
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

fn parse_token_response(status: u16, body: &str) -> Result<TokenSet, TokenError> {
    let json: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    if (200..300).contains(&status) {
        let access_token = json["access_token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| TokenError::Other("token response had no access token".into()))?;
        let s = |k: &str| json[k].as_str().filter(|v| !v.is_empty()).map(str::to_string);
        return Ok(TokenSet {
            access_token: access_token.to_string(),
            refresh_token: s("refresh_token"),
            id_token: s("id_token"),
            expires_in: json["expires_in"].as_u64().unwrap_or(3600),
            scope: s("scope"),
        });
    }
    let code: String = json["error"]
        .as_str()
        .or_else(|| json["error"]["code"].as_str())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_graphic())
        .take(64)
        .collect();
    let desc = json["error_description"]
        .as_str()
        .or_else(|| json["error"]["message"].as_str())
        .and_then(clean_description);
    let oauth = |code: String| OAuthError { status, code, description: desc.clone() };
    if UNUSABLE_CODES.contains(&code.as_str()) {
        return Err(TokenError::Unusable(oauth(code)));
    }
    if code == "invalid_client" {
        return Err(TokenError::InvalidClient(oauth(code)));
    }
    if status == 401 {
        return Err(TokenError::Unusable(oauth(if code.is_empty() { "http_401".into() } else { code })));
    }
    Err(TokenError::Other(match (code.is_empty(), desc) {
        (true, None) => format!("HTTP {status}"),
        (false, None) => format!("HTTP {status} ({code})"),
        (true, Some(d)) => format!("HTTP {status}: {d}"),
        (false, Some(d)) => format!("HTTP {status} ({code}): {d}"),
    }))
}

// ---------------------------------------------------------------------------------------------
// Sign-in state machine (pure; the network and Keychain stay in `sign_in`)
// ---------------------------------------------------------------------------------------------

/// Fresh per-attempt values (sign-in.md §2).
struct Secrets {
    state: String,
    nonce: String,
    verifier: String,
}

impl Secrets {
    fn random() -> Result<Self> {
        Ok(Secrets { state: random_b64(32)?, nonce: random_b64(32)?, verifier: random_b64(32)? })
    }
}

/// One authorization request and what its callback is checked against.
struct Attempt {
    state: String,
    nonce: String,
    verifier: String,
    /// The registration this attempt reauthorizes; None registers a new client.
    registration: Option<Registration>,
}

/// One `sign_in()` call: one listener and redirect URI, at most two authorization attempts.
struct Flow {
    host: String,
    redirect_uri: String,
    attempt: Attempt,
    /// States whose callback has been consumed (code taken for exchange, or the attempt ended).
    /// A repeated request for one of them (reload, prefetch) gets a harmless page, never a second
    /// exchange of the same code.
    spent_states: Vec<String>,
    retried: bool,
}

#[derive(Debug, PartialEq)]
struct Exchange {
    code: String,
    /// The issued client id to exchange with (never `dynamic_agent_client`).
    client_id: String,
    cb_scope: Option<String>,
    /// The attempt registered a new client (authorized with `dynamic_agent_client`).
    new_registration: bool,
}

#[derive(Debug, PartialEq)]
enum CallbackAction {
    /// Not (or no longer) this attempt's callback: answer it and keep waiting.
    Ignore { status: &'static str, title: &'static str, message: &'static str },
    /// The attempt ended without a usable code.
    Fail { code: Option<String>, message: String },
    Exchange(Exchange),
}

enum RetryDecision {
    /// Answer the callback with a redirect to `location` and keep listening.
    Retry { location: String, registration: Registration },
    GiveUp(String),
}

impl Flow {
    fn new(host: String, redirect_uri: String, registration: Option<Registration>, s: Secrets) -> Self {
        Flow {
            host,
            redirect_uri,
            attempt: Attempt { state: s.state, nonce: s.nonce, verifier: s.verifier, registration },
            spent_states: Vec::new(),
            retried: false,
        }
    }

    fn authorize_url(&self) -> String {
        let a = &self.attempt;
        let r = a.registration.as_ref();
        authorize_url(&AuthorizeParams {
            client_id: r.map(|r| r.client_id.as_str()),
            id_token_hint: r.and_then(|r| r.id_token.as_deref()),
            login_hint: r.and_then(|r| r.email.as_deref()),
            host_id: &self.host,
            redirect_uri: &self.redirect_uri,
            state: &a.state,
            nonce: &a.nonce,
            code_challenge: &pkce_challenge(&a.verifier),
        })
    }

    fn on_callback(&mut self, query: &str) -> CallbackAction {
        if callback_state(query).is_some_and(|s| self.spent_states.contains(&s)) {
            log::info!("ChatGPT sign-in: ignored a repeated callback request");
            return CallbackAction::Ignore {
                status: "200 OK",
                title: "Already received",
                message: "Murmur already received this sign-in step. You can close this tab.",
            };
        }
        match parse_callback(query, &self.attempt.state) {
            CallbackOutcome::BadState => {
                // Possibly a stale or forged callback: ignore it and keep waiting for ours.
                log::warn!("ChatGPT sign-in: ignored a callback with a mismatched state");
                CallbackAction::Ignore {
                    status: "400 Bad Request",
                    title: "Sign-in link expired",
                    message: "This sign-in page doesn't match Murmur's current attempt. Start again from Murmur.",
                }
            }
            CallbackOutcome::Error { code, description } => {
                self.spent_states.push(self.attempt.state.clone());
                log::warn!("ChatGPT sign-in: authorization returned error={code}");
                let message = callback_error_message(&code, description.as_deref());
                CallbackAction::Fail { code: Some(code), message }
            }
            CallbackOutcome::Code { code, client_id, scope } => {
                self.spent_states.push(self.attempt.state.clone());
                let saved = self.attempt.registration.as_ref().map(|r| r.client_id.as_str());
                match resolve_client_id(saved, client_id.as_deref()) {
                    Ok(client_id) => CallbackAction::Exchange(Exchange {
                        code,
                        client_id,
                        cb_scope: scope,
                        new_registration: self.attempt.registration.is_none(),
                    }),
                    Err(e) => CallbackAction::Fail { code: None, message: e.to_string() },
                }
            }
        }
    }

    /// sign-in.md §3: "If code exchange returns invalid_grant, discard that code and start a fresh
    /// authorization with the issued client ID retained for this attempt." Once per `sign_in()`,
    /// and only for a new registration (that is where the issued id comes from).
    fn on_unusable(&mut self, ex: &Exchange, err: &OAuthError, s: Secrets) -> RetryDecision {
        if self.retried || !ex.new_registration {
            return RetryDecision::GiveUp(unusable_message(err));
        }
        self.retried = true;
        let registration = Registration { client_id: ex.client_id.clone(), subject: None, email: None, id_token: None };
        self.attempt = Attempt {
            state: s.state,
            nonce: s.nonce,
            verifier: s.verifier,
            registration: Some(registration.clone()),
        };
        RetryDecision::Retry { location: self.authorize_url(), registration }
    }
}

/// The raw `state` of a callback query, if any.
fn callback_state(query: &str) -> Option<String> {
    parse_query(query).into_iter().find(|(k, _)| k == "state").map(|(_, v)| v).filter(|v| !v.is_empty())
}

fn redirect_response(location: &str) -> String {
    let body = page("Continuing sign-in", "Continuing to ChatGPT…");
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn respond_redirect(stream: &mut TcpStream, location: &str) {
    let _ = stream.write_all(redirect_response(location).as_bytes());
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: serde_json::Value) -> String {
        let h = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let p = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("{h}.{p}.")
    }

    fn query_of(url: &str) -> Vec<(String, String)> {
        parse_query(url.split_once('?').unwrap().1)
    }

    fn param<'a>(q: &'a [(String, String)], k: &str) -> Option<&'a str> {
        q.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.as_str())
    }

    #[test]
    fn pkce_matches_rfc7636_appendix_b() {
        let octets = [
            116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212, 37, 77, 105, 214, 191,
            240, 91, 88, 5, 88, 83, 132, 141, 121,
        ];
        let verifier = URL_SAFE_NO_PAD.encode(octets);
        assert_eq!(verifier, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(pkce_challenge(&verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn random_values_are_43_char_base64url() {
        let a = random_b64(32).unwrap();
        assert_eq!(a.len(), 43);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(a, random_b64(32).unwrap());
    }

    fn params<'a>(client_id: Option<&'a str>, hint: Option<&'a str>) -> AuthorizeParams<'a> {
        AuthorizeParams {
            client_id,
            id_token_hint: hint,
            login_hint: client_id.map(|_| "a+b@example.com"),
            host_id: "urn:uuid:1b4e28ba-2fa1-41d2-883f-0016d3cca427",
            redirect_uri: "http://127.0.0.1:1455/auth/callback",
            state: "st",
            nonce: "no",
            code_challenge: "ch",
        }
    }

    #[test]
    fn authorize_url_for_new_registration() {
        let url = authorize_url(&params(None, None));
        assert!(url.starts_with("https://auth.openai.com/api/accounts/authorize?"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback"));
        assert!(url.contains("scope=openid%20profile%20email%20offline_access%20resource.invoke%20chatgpt.tokens.use.direct"));
        let q = query_of(&url);
        assert_eq!(param(&q, "client_id"), Some("dynamic_agent_client"));
        assert_eq!(param(&q, "agent_name_hint"), Some("Murmur"));
        assert_eq!(param(&q, "ext_agent_host_id"), Some("urn:uuid:1b4e28ba-2fa1-41d2-883f-0016d3cca427"));
        assert_eq!(param(&q, "response_type"), Some("code"));
        assert_eq!(param(&q, "resource"), Some("https://api.openai.com/v1"));
        assert_eq!(param(&q, "state"), Some("st"));
        assert_eq!(param(&q, "nonce"), Some("no"));
        assert_eq!(param(&q, "code_challenge"), Some("ch"));
        assert_eq!(param(&q, "code_challenge_method"), Some("S256"));
        assert_eq!(param(&q, "scope"), Some(SCOPES));
        assert_eq!(param(&q, "id_token_hint"), None);
        assert_eq!(param(&q, "login_hint"), None);
    }

    #[test]
    fn authorize_url_for_reauthorization() {
        let q = query_of(&authorize_url(&params(Some("oaiapp_123"), Some("hdr.payload.sig"))));
        assert_eq!(param(&q, "client_id"), Some("oaiapp_123"));
        assert_eq!(param(&q, "agent_name_hint"), None);
        assert_eq!(param(&q, "id_token_hint"), Some("hdr.payload.sig"));
        assert_eq!(param(&q, "login_hint"), Some("a+b@example.com"));
        assert!(param(&q, "ext_agent_host_id").is_some());
        // After sign-out there is no hint.
        let q = query_of(&authorize_url(&params(Some("oaiapp_123"), None)));
        assert_eq!(param(&q, "id_token_hint"), None);
    }

    #[test]
    fn redirect_uses_loopback_ip() {
        assert_eq!(redirect_uri(1455), "http://127.0.0.1:1455/auth/callback");
        assert_eq!(redirect_uri(54321), "http://127.0.0.1:54321/auth/callback");
    }

    #[test]
    fn callback_with_code() {
        let q = "code=abc&scope=chatgpt.tokens.use.direct+email+offline_access+openid+profile+resource.invoke&state=s1&client_id=oaiapp_x";
        assert_eq!(
            parse_callback(q, "s1"),
            CallbackOutcome::Code {
                code: "abc".into(),
                client_id: Some("oaiapp_x".into()),
                scope: Some("chatgpt.tokens.use.direct email offline_access openid profile resource.invoke".into()),
            }
        );
    }

    #[test]
    fn callback_state_mismatch_or_missing() {
        assert_eq!(parse_callback("code=abc&state=other", "s1"), CallbackOutcome::BadState);
        assert_eq!(parse_callback("code=abc", "s1"), CallbackOutcome::BadState);
        // An error with the wrong state is not ours either.
        assert_eq!(parse_callback("error=access_denied&state=x", "s1"), CallbackOutcome::BadState);
    }

    #[test]
    fn callback_error_param() {
        assert_eq!(
            parse_callback("error=access_denied&error_description=User%20declined&state=s1", "s1"),
            CallbackOutcome::Error { code: "access_denied".into(), description: Some("User declined".into()) }
        );
        assert!(callback_error_message("access_denied", None).contains("cancelled"));
        assert!(callback_error_message("server_error", Some("boom")).contains("boom"));
        assert!(matches!(parse_callback("state=s1", "s1"), CallbackOutcome::Error { .. }));
    }

    #[test]
    fn client_id_resolution() {
        assert_eq!(resolve_client_id(None, Some("oaiapp_1")).unwrap(), "oaiapp_1");
        assert!(resolve_client_id(None, None).is_err());
        assert!(resolve_client_id(None, Some("dynamic_agent_client")).is_err());
        assert_eq!(resolve_client_id(Some("oaiapp_1"), None).unwrap(), "oaiapp_1");
        assert_eq!(resolve_client_id(Some("oaiapp_1"), Some("oaiapp_1")).unwrap(), "oaiapp_1");
        assert!(resolve_client_id(Some("oaiapp_1"), Some("oaiapp_2")).is_err());
    }

    #[test]
    fn http_request_parsing() {
        assert_eq!(request_target("GET /auth/callback?code=1 HTTP/1.1\r\nHost: x\r\n\r\n"), Some("/auth/callback?code=1"));
        assert_eq!(request_target("POST /auth/callback HTTP/1.1\r\n"), None);
        assert_eq!(request_target(""), None);
        assert_eq!(split_target("/favicon.ico"), ("/favicon.ico", ""));
        assert_eq!(split_target("/auth/callback?a=b"), ("/auth/callback", "a=b"));
        assert_eq!(percent_decode("a%2Bb+c%"), "a+b c%");
        assert_eq!(percent_decode("%zz%41"), "%zzA");
    }

    #[test]
    fn page_escapes_html() {
        let p = page("Oops", "<script>x</script>");
        assert!(p.contains("&lt;script&gt;") && !p.contains("<script>"));
    }

    #[test]
    fn id_token_valid() {
        let t = jwt(serde_json::json!({
            "iss": "https://auth.openai.com", "aud": ["oaiapp_1"], "exp": 2000, "nonce": "n1",
            "sub": "user-1", "email": "a@example.com"
        }));
        assert_eq!(
            validate_id_token(&t, "oaiapp_1", "n1", 1000).unwrap(),
            IdClaims { sub: "user-1".into(), email: Some("a@example.com".into()) }
        );
        let t = jwt(serde_json::json!({"iss": ISSUER, "aud": "oaiapp_1", "exp": 2000, "nonce": "n1", "sub": "u"}));
        assert_eq!(validate_id_token(&t, "oaiapp_1", "n1", 1000).unwrap().email, None);
    }

    #[test]
    fn id_token_rejections() {
        let base = serde_json::json!({"iss": ISSUER, "aud": "oaiapp_1", "exp": 2000, "nonce": "n1", "sub": "u"});
        let with = |k: &str, v: serde_json::Value| {
            let mut c = base.clone();
            c[k] = v;
            jwt(c)
        };
        let check = |t: &str| validate_id_token(t, "oaiapp_1", "n1", 1000);
        assert!(check(&with("iss", "https://evil.example".into())).is_err());
        assert!(check(&with("aud", "oaiapp_2".into())).is_err());
        assert!(check(&with("aud", serde_json::json!(["x", "y"]))).is_err());
        assert!(check(&with("exp", 500.into())).is_err());
        assert!(check(&with("nonce", "other".into())).is_err());
        assert!(check(&with("sub", "".into())).is_err());
        assert!(check("not-a-jwt").is_err());
        assert!(check("a.!!!.c").is_err());
        assert!(check(&jwt(base.clone())).is_ok());
        // Expired beyond the skew allowance.
        assert!(validate_id_token(&jwt(base), "oaiapp_1", "n1", 2000 + CLOCK_SKEW_SECS).is_err());
    }

    #[test]
    fn token_response_success() {
        let body = r#"{"access_token":"at","refresh_token":"rt","id_token":"it","token_type":"Bearer",
            "expires_in":3600,"scope":"openid chatgpt.tokens.use.direct","earliest_refresh_at":1}"#;
        let t = parse_token_response(200, body).unwrap_or_else(|_| panic!("parse"));
        assert_eq!(t.access_token, "at");
        assert_eq!(t.refresh_token.as_deref(), Some("rt"));
        assert_eq!(t.id_token.as_deref(), Some("it"));
        assert_eq!(t.expires_in, 3600);
        assert!(parse_scopes(t.scope.as_deref().unwrap()).iter().any(|s| s == PLAN_SCOPE));
        assert!(matches!(parse_token_response(200, "{}"), Err(TokenError::Other(_))));
    }

    #[test]
    fn token_response_errors() {
        let e = |status, body| parse_token_response(status, body).err().unwrap();
        let oe = |status, code: &str, d: Option<&str>| OAuthError { status, code: code.into(), description: d.map(Into::into) };
        assert_eq!(e(400, r#"{"error":"invalid_grant"}"#), TokenError::Unusable(oe(400, "invalid_grant", None)));
        assert_eq!(
            e(400, r#"{"error":"invalid_grant","error_description":"Invalid authorization code\n"}"#),
            TokenError::Unusable(oe(400, "invalid_grant", Some("Invalid authorization code")))
        );
        assert_eq!(
            e(400, r#"{"error":{"code":"refresh_token_reused","message":"x"}}"#),
            TokenError::Unusable(oe(400, "refresh_token_reused", Some("x")))
        );
        assert_eq!(e(401, r#"{"error":"invalid_client"}"#), TokenError::InvalidClient(oe(401, "invalid_client", None)));
        assert_eq!(e(401, ""), TokenError::Unusable(oe(401, "http_401", None)));
        assert_eq!(e(503, "oops"), TokenError::Other("HTTP 503".into()));
        assert_eq!(
            e(400, r#"{"error":"invalid_request","error_description":"bad resource"}"#),
            TokenError::Other("HTTP 400 (invalid_request): bad resource".into())
        );
    }

    #[test]
    fn token_error_diagnostics_include_oauth_code() {
        let err = match parse_token_response(400, r#"{"error":"invalid_grant","error_description":"Code expired"}"#) {
            Err(e) => e,
            Ok(_) => panic!("expected an error"),
        };
        assert_eq!(err.log_line(), "HTTP 400, error=invalid_grant, error_description=Code expired");
        let TokenError::Unusable(oauth) = err else { panic!("expected Unusable") };
        let msg = unusable_message(&oauth);
        assert_eq!(msg, "OpenAI rejected the sign-in code (invalid_grant: Code expired). Please try signing in again.");
        // And it reaches the browser page, escaped.
        assert!(page("Sign-in didn't finish", &msg).contains("invalid_grant: Code expired"));
        let bare = OAuthError { status: 400, code: "invalid_grant".into(), description: None };
        assert!(unusable_message(&bare).contains("(invalid_grant)"));
    }

    // --- sign-in state machine ---

    const REDIRECT: &str = "http://127.0.0.1:1455/auth/callback";
    const HOST: &str = "urn:uuid:1b4e28ba-2fa1-41d2-883f-0016d3cca427";

    fn secrets(tag: &str) -> Secrets {
        Secrets { state: format!("state-{tag}"), nonce: format!("nonce-{tag}"), verifier: format!("verifier-{tag}-0123456789012345678901234567") }
    }

    fn new_flow() -> Flow {
        Flow::new(HOST.into(), REDIRECT.into(), None, secrets("1"))
    }

    fn invalid_grant() -> OAuthError {
        OAuthError { status: 400, code: "invalid_grant".into(), description: None }
    }

    #[test]
    fn authorize_url_for_registration_without_id_token() {
        // The registration saved by the invalid_grant recovery: issued client id only.
        let reg = Registration { client_id: "oaiapp_9".into(), subject: None, email: None, id_token: None };
        let flow = Flow::new(HOST.into(), REDIRECT.into(), Some(reg), secrets("r"));
        let url = flow.authorize_url();
        let q = query_of(&url);
        assert_eq!(param(&q, "client_id"), Some("oaiapp_9"));
        assert_eq!(param(&q, "agent_name_hint"), None);
        assert_eq!(param(&q, "id_token_hint"), None);
        assert_eq!(param(&q, "login_hint"), None);
        assert_eq!(param(&q, "ext_agent_host_id"), Some(HOST));
        assert_eq!(param(&q, "redirect_uri"), Some(REDIRECT));
        assert_eq!(param(&q, "state"), Some("state-r"));
        assert_eq!(param(&q, "nonce"), Some("nonce-r"));
        assert_eq!(param(&q, "code_challenge"), Some(pkce_challenge("verifier-r-0123456789012345678901234567").as_str()));
        // Old Keychain records (subject as a plain string, no id_token) still load.
        let old: Registration = serde_json::from_str(r#"{"client_id":"oaiapp_1","subject":"u","email":null}"#).unwrap();
        assert_eq!(old.subject.as_deref(), Some("u"));
        let minimal: Registration = serde_json::from_str(r#"{"client_id":"oaiapp_1"}"#).unwrap();
        assert!(minimal.subject.is_none() && minimal.email.is_none() && minimal.id_token.is_none());
    }

    #[test]
    fn first_attempt_request_matches_exchange() {
        let mut flow = new_flow();
        let q = query_of(&flow.authorize_url());
        let CallbackAction::Exchange(ex) = flow.on_callback("code=oaistb_ac_1&state=state-1&client_id=oaiapp_9") else {
            panic!("expected an exchange")
        };
        // The issued id from the callback, never dynamic_agent_client.
        assert_eq!(ex.client_id, "oaiapp_9");
        assert!(ex.new_registration);
        let form = exchange_form(&ex, &flow.attempt.verifier, &flow.redirect_uri);
        let f = |k: &str| form.iter().find(|(fk, _)| *fk == k).map(|(_, v)| *v);
        assert_eq!(f("grant_type"), Some("authorization_code"));
        assert_eq!(f("client_id"), Some("oaiapp_9"));
        assert_eq!(f("code"), Some("oaistb_ac_1"));
        // Byte-identical redirect_uri and resource; challenge derived from the verifier sent.
        assert_eq!(f("redirect_uri"), param(&q, "redirect_uri"));
        assert_eq!(f("resource"), param(&q, "resource"));
        assert_eq!(Some(pkce_challenge(f("code_verifier").unwrap()).as_str()), param(&q, "code_challenge"));
    }

    #[test]
    fn retry_after_unusable_code_on_new_registration() {
        let mut flow = new_flow();
        let first = flow.authorize_url();
        assert_eq!(param(&query_of(&first), "client_id"), Some(DYNAMIC_CLIENT));

        let CallbackAction::Exchange(ex) = flow.on_callback("code=c1&state=state-1&client_id=oaiapp_9") else {
            panic!("expected an exchange")
        };
        let RetryDecision::Retry { location, registration } = flow.on_unusable(&ex, &invalid_grant(), secrets("2")) else {
            panic!("expected a retry")
        };
        assert_eq!(registration.client_id, "oaiapp_9");
        assert!(registration.subject.is_none() && registration.id_token.is_none() && registration.email.is_none());

        // Reauthorization form with the issued id and fresh values, same redirect URI.
        let q = query_of(&location);
        assert_eq!(param(&q, "client_id"), Some("oaiapp_9"));
        assert_eq!(param(&q, "agent_name_hint"), None);
        assert_eq!(param(&q, "id_token_hint"), None);
        assert_eq!(param(&q, "ext_agent_host_id"), Some(HOST));
        assert_eq!(param(&q, "redirect_uri"), Some(REDIRECT));
        assert_eq!(param(&q, "state"), Some("state-2"));
        assert_eq!(param(&q, "nonce"), Some("nonce-2"));
        assert_eq!(param(&q, "code_challenge"), Some(pkce_challenge(&flow.attempt.verifier).as_str()));
        assert_ne!(param(&q, "code_challenge"), param(&query_of(&first), "code_challenge"));

        // The 302 answer carries the new authorize URL.
        let resp = redirect_response(&location);
        assert!(resp.starts_with("HTTP/1.1 302 Found\r\n"));
        assert!(resp.contains(&format!("\r\nLocation: {location}\r\n")));
        assert!(resp.contains("\r\nReferrer-Policy: no-referrer\r\n"));

        // The old state is not accepted again (and its code is never re-exchanged).
        assert!(matches!(flow.on_callback("code=c1&state=state-1&client_id=oaiapp_9"), CallbackAction::Ignore { .. }));
        // An unknown state is rejected.
        assert!(matches!(
            flow.on_callback("code=cx&state=state-x"),
            CallbackAction::Ignore { status: "400 Bad Request", .. }
        ));

        // The second callback (client_id may be omitted on reauth) exchanges with the issued id.
        let CallbackAction::Exchange(ex2) = flow.on_callback("code=c2&state=state-2") else {
            panic!("expected the second exchange")
        };
        assert_eq!(ex2, Exchange { code: "c2".into(), client_id: "oaiapp_9".into(), cb_scope: None, new_registration: false });
        let form = exchange_form(&ex2, &flow.attempt.verifier, &flow.redirect_uri);
        assert!(form.contains(&("code_verifier", "verifier-2-0123456789012345678901234567")));
        assert!(form.contains(&("redirect_uri", REDIRECT)));
        assert_eq!(flow.attempt.nonce, "nonce-2");

        // A second unusable code gives up, with the OAuth code in the message.
        let RetryDecision::GiveUp(msg) = flow.on_unusable(&ex2, &invalid_grant(), secrets("3")) else {
            panic!("retry must happen at most once")
        };
        assert!(msg.contains("invalid_grant"));
    }

    #[test]
    fn retry_rejects_a_different_client_id() {
        let mut flow = new_flow();
        let CallbackAction::Exchange(ex) = flow.on_callback("code=c1&state=state-1&client_id=oaiapp_9") else { panic!() };
        assert!(matches!(flow.on_unusable(&ex, &invalid_grant(), secrets("2")), RetryDecision::Retry { .. }));
        assert!(matches!(
            flow.on_callback("code=c2&state=state-2&client_id=oaiapp_other"),
            CallbackAction::Fail { code: None, .. }
        ));
    }

    #[test]
    fn no_retry_for_saved_registration() {
        let reg = Registration { client_id: "oaiapp_9".into(), subject: Some("u".into()), email: None, id_token: None };
        let mut flow = Flow::new(HOST.into(), REDIRECT.into(), Some(reg), secrets("1"));
        let CallbackAction::Exchange(ex) = flow.on_callback("code=c1&state=state-1") else { panic!() };
        assert!(!ex.new_registration);
        let RetryDecision::GiveUp(msg) = flow.on_unusable(&ex, &invalid_grant(), secrets("2")) else { panic!() };
        assert!(msg.contains("(invalid_grant)"));
    }

    #[test]
    fn duplicate_callback_is_not_exchanged_twice() {
        let mut flow = new_flow();
        let cb = "code=c1&state=state-1&client_id=oaiapp_9";
        assert!(matches!(flow.on_callback(cb), CallbackAction::Exchange(_)));
        // Reload / prefetch of the same callback URL: harmless page, no second exchange.
        assert_eq!(
            flow.on_callback(cb),
            CallbackAction::Ignore {
                status: "200 OK",
                title: "Already received",
                message: "Murmur already received this sign-in step. You can close this tab.",
            }
        );
        // Same after a callback error.
        let mut flow = new_flow();
        assert!(matches!(flow.on_callback("error=access_denied&state=state-1"), CallbackAction::Fail { .. }));
        assert!(matches!(flow.on_callback("error=access_denied&state=state-1"), CallbackAction::Ignore { .. }));
    }

    #[test]
    fn expiry_logic() {
        assert!(!needs_refresh(1000, 900));
        assert!(needs_refresh(1000, 940));
        assert!(needs_refresh(1000, 1000));
        assert!(needs_refresh(1000, 5000));
    }

    #[test]
    fn plan_usage_from_scopes() {
        let s = |scopes: &str| Session {
            ext_agent_host_id: String::new(),
            client_id: String::new(),
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
            scopes: parse_scopes(scopes),
            email: None,
            subject: String::new(),
            id_token: String::new(),
        };
        assert!(s("openid chatgpt.tokens.use.direct").plan_usage());
        assert!(!s("openid profile email offline_access resource.invoke").plan_usage());
    }
}
