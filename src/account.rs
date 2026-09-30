//! Logging in to the Agent Graph site, for `agent-graph watch-remote`: a
//! share belongs to an account, and only its owner can see it (at /watch).
//!
//! Logging in goes through the browser, where the site signs people in
//! (Google, or an email and password), as an OAuth client on the same
//! computer would (RFC 8252), with PKCE (RFC 7636):
//!
//! 1. This listens on a port of 127.0.0.1 and opens the site's
//!    `/login?cli=<port>&state=<random>&challenge=<SHA-256 of a secret>`.
//! 2. Once logged in there, the site sends the browser to
//!    `http://127.0.0.1:<port>/callback?code=<one-time code>&state=…`.
//! 3. This trades the code, with the secret (which never left this program
//!    until now), for a token of its own (`POST /api/cli/token`), and keeps
//!    it in the data folder (`account.json`, readable only by the user).
//!    The page then says it worked, and can be closed.
//!
//! The token is sent to make shares. `--logout` has the site forget it, and
//! removes the file.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::remote::{Client, base64url};

/// How long to wait for the login to be finished in the browser.
const LOGIN_WAIT: Duration = Duration::from_secs(10 * 60);

/// Being logged in to `site`, as `email`, with the token it gave this computer.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Account {
    pub site: String,
    pub token: String,
    pub email: Option<String>,
}

/// Where an account stands, as the site says when a share's started with
/// its login: an account that hasn't subscribed can share live for its
/// first days, and then not till it does (the site's `lib/billing.ts`).
#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AccountStatus {
    Active,
    Unpaid,
    /// One this version doesn't know: the site's newer.
    #[serde(other)]
    Unknown,
}

impl Account {
    /// Who it is, for messages.
    pub fn who(&self) -> &str {
        self.email.as_deref().unwrap_or("your account")
    }
}

fn account_path(root: &Path) -> PathBuf {
    root.join("account.json")
}

/// The saved login for `site`, if there is one.
pub fn load(root: &Path, site: &str) -> Option<Account> {
    let text = std::fs::read_to_string(account_path(root)).ok()?;
    let account: Account = serde_json::from_str(&text).ok()?;
    (account.site == site).then_some(account)
}

fn save(root: &Path, account: &Account) -> Result<(), String> {
    let path = account_path(root);
    let text = serde_json::to_string_pretty(account).expect("serializable");
    crate::remote::write_private(&path, text.as_bytes())
        .map_err(|e| format!("saving the login in {}: {e}", path.display()))
}

/// The environment variable an API token is given in: for a machine that
/// can't log in in a browser (a cloud instance, say). One's made on the
/// site's account page.
pub const TOKEN_VAR: &str = "AGENT_GRAPH_TOKEN";

/// Why `AGENT_GRAPH_TOKEN` couldn't be used.
pub enum TokenError {
    /// The site doesn't know it: revoked, or mistyped. Trying again won't help.
    Unknown(String),
    /// The site couldn't be asked, or the login couldn't be saved.
    Other(String),
}

impl TokenError {
    pub fn message(self) -> String {
        match self {
            TokenError::Unknown(m) | TokenError::Other(m) => m,
        }
    }
}

/// The login `AGENT_GRAPH_TOKEN` gives, if it's set: the site's asked whose
/// it is (so a wrong one is said now, not when sharing starts), and it's
/// saved as this computer's login, so a run started without it (by
/// `--autostart`'s service, say) has it too. One saved already, with the
/// same token, is used as it is.
pub fn from_env(root: &Path, site: &str, client: &Client) -> Result<Option<Account>, TokenError> {
    let Some(token) = std::env::var(TOKEN_VAR)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
    else {
        return Ok(None);
    };
    if let Some(saved) = load(root, site).filter(|a| a.token == token) {
        return Ok(Some(saved));
    }
    let email = client.account_email(&token).map_err(|e| match e {
        crate::remote::SendError::LoggedOut(_) => TokenError::Unknown(format!(
            "{TOKEN_VAR} isn't a login {site} knows: it may have been revoked. Make one on your account page, {site}/account."
        )),
        e => TokenError::Other(format!("checking {TOKEN_VAR} with {site}: {}", e.message())),
    })?;
    let account = Account {
        site: site.to_string(),
        token,
        email,
    };
    save(root, &account).map_err(TokenError::Other)?;
    Ok(Some(account))
}

/// Forgets the saved login. Whether there was one.
pub fn forget(root: &Path) -> Result<bool, String> {
    let path = account_path(root);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("removing {}: {e}", path.display())),
    }
}

/// Logs out: the site forgets this computer's token, and the saved login
/// goes. (If the site can't be reached, the login's still removed here; it
/// can be removed from the site on the account page.)
pub fn logout(root: &Path, site: &str) -> Result<(), String> {
    let saved = std::fs::read_to_string(account_path(root))
        .ok()
        .and_then(|text| serde_json::from_str::<Account>(&text).ok());
    let Some(account) = saved else {
        forget(root)?;
        eprintln!("Not logged in.");
        return Ok(());
    };
    // With the site it was made for, whatever --url says.
    let site = if account.site == site {
        site
    } else {
        &account.site
    };
    if let Err(e) = Client::new(site).cli_logout(&account.token) {
        eprintln!(
            "Couldn't tell {site} ({}). To be sure the login's ended there, remove this computer on your account page: {site}/account",
            e.message()
        );
    }
    forget(root)?;
    eprintln!("Logged out of {site} ({}).", account.who());
    Ok(())
}

/// Logs in through the browser (see the module docs), and saves the login.
pub fn login(root: &Path, site: &str, client: &Client) -> Result<Account, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("can't wait for the login on this computer: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let state = random_secret()?;
    let verifier = random_secret()?;
    let challenge =
        base64url(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref());
    let url = format!("{site}/login?cli={port}&state={state}&challenge={challenge}");

    eprintln!("To share, log in to {site}. Opening your browser at:\n  {url}");
    eprintln!("(If it doesn't open, open that address yourself.)");
    if std::env::var_os("AGENT_GRAPH_NO_BROWSER").is_none() {
        crate::view::open_browser(&url);
    }

    let deadline = Instant::now() + LOGIN_WAIT;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    loop {
        if Instant::now() > deadline {
            return Err("the login wasn't finished in time. Run it again to try again.".into());
        }
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(format!("waiting for the login: {e}")),
        };
        // A request that isn't the callback (a favicon, say) is answered and
        // waited past; the callback is answered with how it went.
        if let Some(account) = callback(stream, &state, &verifier, site, client)? {
            save(root, &account)?;
            eprintln!("Logged in as {}.", account.who());
            return Ok(account);
        }
    }
}

/// Answers one request to the login's port. The account, once a callback
/// with this login's `state` has been traded for a token; an error if the
/// site refused it (the page says so too).
fn callback(
    mut stream: TcpStream,
    state: &str,
    verifier: &str,
    site: &str,
    client: &Client,
) -> Result<Option<Account>, String> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return Ok(None);
    }
    let target = line.split(' ').nth(1).unwrap_or("");
    let Some(query) = target.strip_prefix("/callback?") else {
        reply(
            &mut stream,
            404,
            "Not found",
            "This is agent-graph, waiting for you to log in.",
        );
        return Ok(None);
    };
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| percent_decode(v))
    };
    if param("state").as_deref() != Some(state) {
        reply(
            &mut stream,
            400,
            "Not this login",
            "This isn't the login agent-graph is waiting for. Go back to the terminal, and follow the address it shows.",
        );
        return Ok(None);
    }
    let Some(code) = param("code") else {
        reply(
            &mut stream,
            400,
            "Something's missing",
            "The login didn't come back complete. Try again.",
        );
        return Ok(None);
    };
    match client.cli_token(&code, verifier, &host_name()) {
        Ok((token, email)) => {
            let account = Account {
                site: site.to_string(),
                token,
                email,
            };
            reply(
                &mut stream,
                200,
                "Logged in",
                &format!(
                    "agent-graph is logged in as {}, and is sharing now. You can close this window.",
                    html_escape(account.who())
                ),
            );
            Ok(Some(account))
        }
        Err(e) => {
            let e = e.message();
            reply(&mut stream, 502, "That didn't work", &html_escape(&e));
            Err(format!("logging in: {e}"))
        }
    }
}

fn reply(stream: &mut TcpStream, status: u16, title: &str, text: &str) {
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title} · Agent Graph</title></head><body style=\"font-family: system-ui, sans-serif; max-width: 34em; margin: 12vh auto; padding: 0 20px; line-height: 1.5\"><h1 style=\"font-size: 1.4em\">{title}</h1><p>{text}</p></body></html>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {title}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

/// 32 random bytes, as base64url (43 characters).
fn random_secret() -> Result<String, String> {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "couldn't get random bytes from the system".to_string())?;
    Ok(base64url(&bytes))
}

/// What to call this computer on the account page and at /watch: its host
/// name, or, in Claude Code's cloud (whose containers are all called "vm"),
/// "Claude Code cloud".
pub fn host_name() -> String {
    if in_claude_code_cloud(std::env::var("CLAUDE_CODE_REMOTE").ok().as_deref()) {
        return CLAUDE_CODE_CLOUD.to_string();
    }
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default()
}

const CLAUDE_CODE_CLOUD: &str = "Claude Code cloud";

/// Whether CLAUDE_CODE_REMOTE says this is Claude Code's cloud.
fn in_claude_code_cloud(remote: Option<&str>) -> bool {
    remote == Some("true")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_claude_code_cloud_is_called_so() {
        assert!(in_claude_code_cloud(Some("true")));
        assert!(!in_claude_code_cloud(Some("false")));
        assert!(!in_claude_code_cloud(Some("")));
        assert!(!in_claude_code_cloud(None));
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%2Fb+c"), "a/b c");
        assert_eq!(percent_decode("abc"), "abc");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn secrets_are_43_characters_of_base64url() {
        let a = random_secret().unwrap();
        let b = random_secret().unwrap();
        assert_eq!(a.len(), 43);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(a, b);
    }

    #[test]
    fn account_statuses() {
        let parse = |s: &str| serde_json::from_str::<AccountStatus>(s).unwrap();
        assert_eq!(parse(r#""active""#), AccountStatus::Active);
        assert_eq!(parse(r#""unpaid""#), AccountStatus::Unpaid);
        assert_eq!(parse(r#""something-new""#), AccountStatus::Unknown);
    }

    #[test]
    fn a_login_is_only_for_its_site() {
        let root = tempfile::tempdir().unwrap();
        let account = Account {
            site: "https://a.example".into(),
            token: "agt_x".into(),
            email: Some("me@example.com".into()),
        };
        save(root.path(), &account).unwrap();
        assert_eq!(load(root.path(), "https://a.example"), Some(account));
        assert_eq!(load(root.path(), "https://b.example"), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(account_path(root.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "only its owner can read the token");
        }
        assert!(forget(root.path()).unwrap());
        assert!(!forget(root.path()).unwrap());
    }
}
