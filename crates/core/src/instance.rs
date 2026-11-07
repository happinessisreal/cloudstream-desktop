//! Single-instance routing (T27).
//!
//! The first launch becomes the *primary*: it holds an exclusive lock and listens on a private
//! Unix socket. A later launch (a clicked link, "open with", a second start from the menu) becomes
//! a *secondary*: it sends its arguments to the primary, waits for an acknowledgement, and exits.
//!
//! The lock decides who is primary without races; the socket only carries the arguments. The
//! socket lives in a user-only directory, every connection is checked against the caller's user
//! id, and messages are small and bounded. The primary treats everything it receives as untrusted
//! text and passes it to `entry::parse_arguments`; nothing is executed because of it.
use crate::entry::MAX_ARGUMENT_BYTES;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    time::Duration,
};

pub const MAX_FORWARDED_ARGUMENTS: usize = 32;
const MAX_MESSAGE_BYTES: usize = 128 * 1024;
const LOCK_NAME: &str = "cloudstream-desktop.lock";
const SOCKET_NAME: &str = "cloudstream-desktop.sock";

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    version: u32,
    arguments: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub enum Role {
    Primary(Primary),
    /// Another instance is running; its address is the directory to forward to.
    Secondary,
}

pub struct Primary {
    _lock: File,
    listener: UnixListener,
    socket: PathBuf,
}

/// A user-only directory for the lock and socket: `$XDG_RUNTIME_DIR`, else a per-user folder.
pub fn runtime_directory() -> Result<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let base = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_dir()) {
        Some(dir) => dir.join("cloudstream-desktop"),
        None => std::env::temp_dir().join(format!("cloudstream-desktop-{uid}")),
    };
    ensure_private_directory(&base)?;
    Ok(base)
}

pub fn ensure_private_directory(path: &Path) -> Result<()> {
    match std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) => return Err(error).with_context(|| format!("Could not create {}", path.display())),
    }
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        bail!("{} is not a plain folder", path.display());
    }
    if meta.uid() != unsafe { libc::geteuid() } {
        bail!("{} belongs to another user", path.display());
    }
    if meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Decide whether this process is the primary. `directory` must be a private folder.
pub fn acquire(directory: &Path) -> Result<Role> {
    ensure_private_directory(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join(LOCK_NAME))
        .context("Could not open the instance lock")?;
    let meta = lock.metadata()?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } {
        bail!("The instance lock is not safe to use");
    }
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(Role::Secondary);
    }
    // Holding the lock proves no live primary exists, so a leftover socket is stale.
    let socket = directory.join(SOCKET_NAME);
    match std::fs::symlink_metadata(&socket) {
        Ok(meta) if meta.file_type().is_socket_like() => std::fs::remove_file(&socket)?,
        Ok(_) => bail!("{} exists and is not a socket", socket.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Could not inspect the instance socket"),
    }
    let listener = UnixListener::bind(&socket).context("Could not listen for other launches")?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    Ok(Role::Primary(Primary { _lock: lock, listener, socket }))
}

trait SocketLike {
    fn is_socket_like(&self) -> bool;
}
impl SocketLike for std::fs::FileType {
    fn is_socket_like(&self) -> bool {
        std::os::unix::fs::FileTypeExt::is_socket(self)
    }
}

fn peer_is_same_user(stream: &UnixStream) -> bool {
    let mut credentials = libc::ucred { pid: 0, uid: u32::MAX, gid: 0 };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut credentials as *mut libc::ucred).cast(), &mut length)
    };
    result == 0 && credentials.uid == unsafe { libc::geteuid() }
}

fn validate(request: &Request) -> Result<()> {
    if request.version != 1 {
        bail!("Unsupported request version");
    }
    if request.arguments.len() > MAX_FORWARDED_ARGUMENTS {
        bail!("Too many arguments");
    }
    if request.arguments.iter().any(|a| a.len() > MAX_ARGUMENT_BYTES) {
        bail!("An argument is too long");
    }
    Ok(())
}

fn handle_connection(mut stream: UnixStream, handler: &(dyn Fn(Vec<String>) + Send + Sync)) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let reply = (|| -> Result<()> {
        if !peer_is_same_user(&stream) {
            bail!("Not allowed");
        }
        let mut line = String::new();
        // Bounded read: at most MAX_MESSAGE_BYTES, ending at the first newline.
        let mut reader = BufReader::new((&stream).take(MAX_MESSAGE_BYTES as u64 + 1));
        reader.read_line(&mut line).context("Could not read the request")?;
        if line.len() > MAX_MESSAGE_BYTES || !line.ends_with('\n') {
            bail!("The request is too large or incomplete");
        }
        let request: Request = serde_json::from_str(line.trim_end()).context("The request is not valid")?;
        validate(&request)?;
        handler(request.arguments);
        Ok(())
    })();
    let reply = match reply {
        Ok(()) => Reply { ok: true, error: None },
        Err(error) => Reply { ok: false, error: Some(error.to_string()) },
    };
    if let Ok(mut text) = serde_json::to_string(&reply) {
        text.push('\n');
        let _ = stream.write_all(text.as_bytes());
    }
}

impl Primary {
    /// Serve other launches on a background thread. The handler gets each launch's arguments and
    /// must not block for long; a panic in it does not stop the listener.
    pub fn serve(self, handler: impl Fn(Vec<String>) + Send + Sync + 'static) -> std::thread::JoinHandle<()> {
        let handler = std::sync::Arc::new(handler);
        std::thread::spawn(move || {
            let _keep_lock_alive = &self._lock;
            for stream in self.listener.incoming() {
                let Ok(stream) = stream else { continue };
                let handler = handler.clone();
                // One short-lived thread per connection, so a slow peer cannot block the next.
                std::thread::spawn(move || {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle_connection(stream, handler.as_ref())));
                });
            }
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }
}

impl Drop for Primary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// Make relative file arguments absolute using `cwd`, so the primary (which has a different
/// working directory) opens the file the user meant. Links, flags and absolute paths are kept.
pub fn absolutize_arguments(arguments: &[String], cwd: &Path) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| {
            let is_flag = argument.starts_with('-');
            let has_scheme = argument.find(':').is_some_and(|i| i > 1 && argument[..i].bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')));
            if is_flag || has_scheme || argument.is_empty() || Path::new(argument).is_absolute() {
                argument.clone()
            } else {
                cwd.join(argument).display().to_string()
            }
        })
        .collect()
}

/// Send this launch's arguments to the running primary. Retries briefly in case the primary holds
/// the lock but has not started listening yet.
pub fn forward(directory: &Path, arguments: &[String]) -> Result<()> {
    let request = Request { version: 1, arguments: arguments.to_vec() };
    validate(&request)?;
    let mut text = serde_json::to_string(&request)?;
    text.push('\n');
    let socket = directory.join(SOCKET_NAME);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut stream = loop {
        match UnixStream::connect(&socket) {
            Ok(stream) => break stream,
            Err(error) if std::time::Instant::now() < deadline => {
                let _ = error;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error).context("The running instance did not answer"),
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(text.as_bytes())?;
    let mut answer = String::new();
    BufReader::new(&stream).read_line(&mut answer).context("The running instance did not acknowledge")?;
    let reply: Reply = serde_json::from_str(answer.trim_end()).context("The running instance sent an unreadable answer")?;
    if !reply.ok {
        bail!("The running instance refused the request: {}", reply.error.unwrap_or_default());
    }
    Ok(())
}
