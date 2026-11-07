//! Where service credentials live: the operating system's keyring (Secret Service on Linux),
//! reached through the `secret-tool` program that ships with libsecret. The secret is passed on
//! standard input, never on a command line, and nothing is ever written to the app's own files.
//!
//! When no keyring is available the answer is "not available", and connecting is refused. There
//! is deliberately no plain-text fallback.
use super::{BoxFuture, Credentials};
use std::{collections::HashMap, sync::Mutex, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// `secret-tool` is not installed.
    NotInstalled,
    /// No keyring service answered, or it is locked and was not unlocked.
    Unavailable,
    Failed,
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => f.write_str("Secure storage needs the libsecret tools (the secret-tool program). Install them, then try again."),
            Self::Unavailable => f.write_str("No keyring is available. Start a keyring service such as GNOME Keyring or KWallet and unlock it, then try again."),
            Self::Failed => f.write_str("The keyring could not save or read the secret."),
        }
    }
}
impl std::error::Error for SecretError {}

pub trait SecretStore: Send + Sync {
    fn get<'a>(&'a self, provider: &'a str, profile: &'a str) -> BoxFuture<'a, Result<Option<Credentials>, SecretError>>;
    fn set<'a>(&'a self, provider: &'a str, profile: &'a str, credentials: &'a Credentials) -> BoxFuture<'a, Result<(), SecretError>>;
    fn delete<'a>(&'a self, provider: &'a str, profile: &'a str) -> BoxFuture<'a, Result<(), SecretError>>;
    /// Whether a keyring can be used right now.
    fn available<'a>(&'a self) -> BoxFuture<'a, Result<(), SecretError>>;
}

fn encode(credentials: &Credentials) -> String {
    serde_json::to_string(credentials).unwrap_or_default()
}

fn decode(text: &str) -> Option<Credentials> {
    serde_json::from_str(text.trim()).ok()
}

/// The profile id and provider id become keyring attributes; only plain identifiers are accepted.
fn plain(text: &str) -> bool {
    !text.is_empty() && text.len() <= 64 && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub struct SecretTool {
    program: std::path::PathBuf,
    env: Vec<(String, String)>,
}

impl SecretTool {
    pub fn new() -> Self {
        Self { program: "secret-tool".into(), env: Vec::new() }
    }
    /// Use a different program (tests supply a stand-in).
    pub fn with_program(program: impl Into<std::path::PathBuf>) -> Self {
        Self { program: program.into(), env: Vec::new() }
    }
    pub fn with_env(mut self, name: &str, value: impl Into<String>) -> Self {
        self.env.push((name.into(), value.into()));
        self
    }

    async fn run(&self, args: Vec<String>, stdin: Option<String>) -> Result<(std::process::ExitStatus, String, String), SecretError> {
        use tokio::io::AsyncWriteExt;
        let mut command = tokio::process::Command::new(&self.program);
        command.args(&args).envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str()))).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { SecretError::NotInstalled } else { SecretError::Failed })?;
        if let Some(mut pipe) = child.stdin.take() {
            if let Some(text) = stdin {
                pipe.write_all(text.as_bytes()).await.map_err(|_| SecretError::Failed)?;
            }
            drop(pipe);
        }
        // An unlock prompt may be showing; give the person time to answer it.
        let output = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output()).await.map_err(|_| SecretError::Unavailable)?.map_err(|_| SecretError::Failed)?;
        Ok((output.status, String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned()))
    }

    fn attributes(provider: &str, profile: &str) -> Result<Vec<String>, SecretError> {
        if !plain(provider) || !plain(profile) {
            return Err(SecretError::Failed);
        }
        Ok(["application", "cloudstream-desktop", "kind", "subtitle-service", "service", provider, "profile", profile].map(String::from).to_vec())
    }
}

impl Default for SecretTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for SecretTool {
    fn get<'a>(&'a self, provider: &'a str, profile: &'a str) -> BoxFuture<'a, Result<Option<Credentials>, SecretError>> {
        Box::pin(async move {
            let mut args = vec!["lookup".to_string()];
            args.extend(Self::attributes(provider, profile)?);
            let (status, stdout, stderr) = self.run(args, None).await?;
            if status.success() {
                return Ok(decode(&stdout));
            }
            // Exit 1 with nothing on stderr is "no such item"; anything on stderr is a keyring problem.
            if stderr.trim().is_empty() {
                Ok(None)
            } else {
                Err(SecretError::Unavailable)
            }
        })
    }

    fn set<'a>(&'a self, provider: &'a str, profile: &'a str, credentials: &'a Credentials) -> BoxFuture<'a, Result<(), SecretError>> {
        Box::pin(async move {
            let mut args = vec!["store".to_string(), format!("--label=Cloudstream Desktop: {provider} ({profile})")];
            args.extend(Self::attributes(provider, profile)?);
            let (status, _, stderr) = self.run(args, Some(encode(credentials))).await?;
            match (status.success(), stderr.trim().is_empty()) {
                (true, _) => Ok(()),
                (false, false) => Err(SecretError::Unavailable),
                (false, true) => Err(SecretError::Failed),
            }
        })
    }

    fn delete<'a>(&'a self, provider: &'a str, profile: &'a str) -> BoxFuture<'a, Result<(), SecretError>> {
        Box::pin(async move {
            let mut args = vec!["clear".to_string()];
            args.extend(Self::attributes(provider, profile)?);
            let (status, _, stderr) = self.run(args, None).await?;
            if status.success() || stderr.trim().is_empty() {
                Ok(())
            } else {
                Err(SecretError::Unavailable)
            }
        })
    }

    fn available<'a>(&'a self) -> BoxFuture<'a, Result<(), SecretError>> {
        Box::pin(async move {
            let args = ["lookup", "application", "cloudstream-desktop", "kind", "availability-check"].map(String::from).to_vec();
            let (status, _, stderr) = self.run(args, None).await?;
            if status.success() || stderr.trim().is_empty() {
                Ok(())
            } else {
                Err(SecretError::Unavailable)
            }
        })
    }
}

/// An in-memory store for tests; `down` simulates a missing keyring.
#[derive(Default)]
pub struct MemorySecrets {
    items: Mutex<HashMap<(String, String), Credentials>>,
    pub down: std::sync::atomic::AtomicBool,
}

impl MemorySecrets {
    fn check(&self) -> Result<(), SecretError> {
        if self.down.load(std::sync::atomic::Ordering::SeqCst) {
            Err(SecretError::Unavailable)
        } else {
            Ok(())
        }
    }
    pub fn snapshot(&self) -> HashMap<(String, String), Credentials> {
        self.items.lock().unwrap().clone()
    }
}

impl SecretStore for MemorySecrets {
    fn get<'a>(&'a self, provider: &'a str, profile: &'a str) -> BoxFuture<'a, Result<Option<Credentials>, SecretError>> {
        Box::pin(async move {
            self.check()?;
            Ok(self.items.lock().unwrap().get(&(provider.into(), profile.into())).cloned())
        })
    }
    fn set<'a>(&'a self, provider: &'a str, profile: &'a str, credentials: &'a Credentials) -> BoxFuture<'a, Result<(), SecretError>> {
        Box::pin(async move {
            self.check()?;
            self.items.lock().unwrap().insert((provider.into(), profile.into()), credentials.clone());
            Ok(())
        })
    }
    fn delete<'a>(&'a self, provider: &'a str, profile: &'a str) -> BoxFuture<'a, Result<(), SecretError>> {
        Box::pin(async move {
            self.check()?;
            self.items.lock().unwrap().remove(&(provider.into(), profile.into()));
            Ok(())
        })
    }
    fn available<'a>(&'a self) -> BoxFuture<'a, Result<(), SecretError>> {
        Box::pin(async move { self.check() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in `secret-tool` that keeps items in a folder, honours the same argument and
    /// exit-code conventions (lookup: secret on stdout, exit 1 and silence when missing), and
    /// records its command lines so a test can prove the secret never appears on one.
    fn stand_in(dir: &std::path::Path) -> std::path::PathBuf {
        let script = dir.join("secret-tool");
        std::fs::write(
            &script,
            r#"#!/bin/sh
echo "$@" >> "$STORE/argv.log"
cmd="$1"; shift
[ "$cmd" = store ] && shift
key=$(printf '%s\n' "$@" | tr '/ ' '__' | tr -d '\n')
file="$STORE/item-$key"
case "$cmd" in
  store) cat > "$file" ;;
  lookup) [ -f "$STORE/down" ] && { echo "secret-tool: Cannot autolaunch D-Bus" >&2; exit 1; }; [ -f "$file" ] && { cat "$file"; exit 0; }; exit 1 ;;
  clear) rm -f "$file"; exit 0 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn creds(pairs: &[(&str, &str)]) -> Credentials {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[tokio::test]
    async fn secrets_round_trip_through_the_program_and_never_touch_a_command_line() {
        let dir = tempfile::tempdir().unwrap();
        let tool = SecretTool::with_program(stand_in(dir.path())).with_env("STORE", dir.path().to_string_lossy());
        assert_eq!(tool.get("subdl", "default").await, Ok(None), "a missing item is not an error");
        let saved = creds(&[("apiKey", "SECRET-KEY-123"), ("password", "p\"a'ss word")]);
        tool.set("subdl", "default", &saved).await.unwrap();
        assert_eq!(tool.get("subdl", "default").await, Ok(Some(saved)));
        assert_eq!(tool.get("subdl", "other-profile").await, Ok(None), "profiles do not share credentials");
        assert_eq!(tool.get("subsource", "default").await, Ok(None), "services do not share credentials");
        tool.delete("subdl", "default").await.unwrap();
        assert_eq!(tool.get("subdl", "default").await, Ok(None));
        let log = std::fs::read_to_string(dir.path().join("argv.log")).unwrap();
        assert!(!log.contains("SECRET-KEY") && !log.contains("p\"a'ss"), "secrets only travel on standard input:\n{log}");
        assert!(log.contains("application cloudstream-desktop"));
    }

    #[tokio::test]
    async fn a_missing_program_or_keyring_is_reported_not_papered_over() {
        let tool = SecretTool::with_program("/nonexistent/secret-tool");
        assert_eq!(tool.available().await, Err(SecretError::NotInstalled));
        assert_eq!(tool.set("subdl", "default", &creds(&[("apiKey", "k")])).await, Err(SecretError::NotInstalled));

        let dir = tempfile::tempdir().unwrap();
        let tool = SecretTool::with_program(stand_in(dir.path())).with_env("STORE", dir.path().to_string_lossy());
        assert_eq!(tool.available().await, Ok(()));
        std::fs::write(dir.path().join("down"), b"").unwrap();
        assert_eq!(tool.available().await, Err(SecretError::Unavailable));
        assert_eq!(tool.get("subdl", "default").await, Err(SecretError::Unavailable));
    }

    #[tokio::test]
    async fn only_plain_identifiers_become_keyring_attributes() {
        let tool = SecretTool::with_program("/nonexistent/secret-tool");
        for bad in ["", "a b", "x/../y", "n\nl", &"a".repeat(65)] {
            assert_eq!(tool.get(bad, "default").await, Err(SecretError::Failed), "{bad:?}");
            assert_eq!(tool.get("subdl", bad).await, Err(SecretError::Failed), "{bad:?}");
        }
    }
}
