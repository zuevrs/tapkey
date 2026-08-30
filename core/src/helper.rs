//! The credential helper, as a library so its logic is testable without spawning it.
//!
//! The contract was earned on Claude Code and re-measured on Codex, and it is narrow: **stdout
//! carries the credential and nothing else**, failure is a non-zero exit, and diagnostics go to
//! stderr, which Codex ignores. A one-line banner ahead of the token removed the authorization
//! header entirely in both tools, with nothing anywhere naming credentials — only a `401` the
//! person blames on their provider. `get` therefore writes the secret and stops.
//!
//! Three exit codes, because the core's outcomes differ: `0` found, `1` no such item, `2` refused
//! or broken. One non-zero code for the last two would make the core guess, and the guess becomes
//! *add a key* said to somebody whose key exists and was withheld.
//!
//! The secret arrives on **stdin** and never as an argument, because arguments are visible in the
//! process list to anyone on the machine. It is held in one buffer for the length of one call and
//! written nowhere else — not to a log, not to an error message, not into a panic.

use std::path::PathBuf;

/// Where the helper's run is told to go.
pub struct Io {
    /// The subcommand and its argument, already separated.
    pub args: Vec<String>,
    /// Everything on stdin, which is where `set`'s secret arrives.
    pub stdin: Vec<u8>,
    /// Where `get`'s answer goes.
    pub stdout: Vec<u8>,
    /// Where diagnostics go. Codex ignores this; that is measured, and it is what makes it safe.
    pub stderr: Vec<u8>,
}

impl Io {
    fn say(&mut self, line: &str) {
        self.stderr.extend_from_slice(line.as_bytes());
        self.stderr.push(b'\n');
    }
}

/// Which of the run's subcommands it was.
enum Command {
    Get(String),
    Has(String),
    Set(String),
    Forget(String),
}

fn parse(args: &[String]) -> Result<Command, &'static str> {
    let (name, id) = match (args.first().map(String::as_str), args.get(1)) {
        (Some(name), Some(id)) if !id.is_empty() => (name, id),
        _ => return Err("usage: tapkey-helper <get|has|set|forget> <provider>"),
    };
    Ok(match name {
        "get" => Command::Get(id.clone()),
        "has" => Command::Has(id.clone()),
        "set" => Command::Set(id.clone()),
        "forget" => Command::Forget(id.clone()),
        _ => return Err("usage: tapkey-helper <get|has|set|forget> <provider>"),
    })
}

/// The whole of a run, reduced to the exit code that is all the caller sees on `has`.
pub fn run(io: &mut Io) -> i32 {
    let command = match parse(&io.args) {
        Ok(c) => c,
        Err(why) => {
            io.say(why);
            return 2;
        }
    };

    // `set` reads its secret from stdin, which is the only channel nobody else on the machine can
    // read while it happens. An empty secret is refused rather than stored: storing nothing under a
    // name that looks stored would turn `has` into a lie.
    let secret = match &command {
        Command::Set(_) => {
            if io.stdin.is_empty() {
                io.say("a secret is expected on stdin");
                return 2;
            }
            io.stdin.clone()
        }
        _ => Vec::new(),
    };

    match perform(&command, &secret, io) {
        Ok(stored) if stored => 0,
        Ok(_) => 1,
        Err(why) => {
            io.say(&why);
            2
        }
    }
}

/// Do the work. `Ok(true)` means the answer was yes — found, stored, forgotten; `Ok(false)` means
/// the honest answer was *no such item*, which is exit 1 and not an error.
fn perform(command: &Command, secret: &[u8], io: &mut Io) -> Result<bool, String> {
    match command {
        Command::Get(provider) => match Store::open()?.read(provider)? {
            Some(secret) => {
                // No trailing newline. Codex is measured to trim both ends of what it reads; Claude
                // Code is unmeasured, so printing none is the side on which being wrong is harmless.
                io.stdout.extend_from_slice(&secret);
                Ok(true)
            }
            None => Ok(false),
        },
        Command::Has(provider) => Ok(Store::open()?.read(provider)?.is_some()),
        Command::Set(provider) => {
            Store::open()?.write(provider, secret)?;
            Ok(true)
        }
        Command::Forget(provider) => {
            let store = Store::open()?;
            if store.read(provider)?.is_none() {
                return Ok(false);
            }
            store.remove(provider)?;
            Ok(true)
        }
    }
}

// -- Where secrets live ------------------------------------------------------------------

/// One secret store, whichever kind this platform has.
enum Store {
    /// The file-backed kind, at `<root>/<provider>`, created at `0600`.
    ///
    /// This is the **only** kind on Linux — ADR-0007 prescribes it there, and it is the same branch
    /// OpenCode takes on every platform. A test selects it explicitly with `TAPKEY_FILE_STORE`,
    /// which is how the whole surface of subcommands is exercised without going near a real
    /// Keychain.
    File(PathBuf),
    /// The platform's own credential store, through `keyring`. The entry is built per operation,
    /// because the account **is** the provider id and `open` does not know it yet.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    Platform,
}

impl Store {
    /// The store this run uses.
    ///
    /// The platform decides: its own credential store where one is wired, the file-backed kind
    /// where none is. `TAPKEY_STORE` names **where** files go when files are used — it is not a
    /// vote for them: for the first shipped iteration a real macOS key landed in a plaintext
    /// file because the path variable doubled as the selector, and the catalogue promises the
    /// Keychain. `TAPKEY_FILE_STORE` is the explicit opt-out that lets a test point every part
    /// of the machinery at a directory it owns, on any platform.
    fn open() -> Result<Store, String> {
        let file_root = || -> PathBuf {
            match std::env::var_os("TAPKEY_STORE") {
                Some(root) => {
                    let mut path = PathBuf::from(root);
                    path.push("keys");
                    path
                }
                None => default_file_root(),
            }
        };
        if std::env::var_os("TAPKEY_FILE_STORE").is_some_and(|v| v != "0") {
            return Ok(Store::File(file_root()));
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            // The service and account are compatibility surface forever: renaming them later would
            // orphan every stored secret. The account is the provider id, not the base URL, because
            // two providers may share one — a work key and a personal one.
            Ok(Store::Platform)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Ok(Store::File(file_root()))
        }
    }

    fn read(&self, provider: &str) -> Result<Option<Vec<u8>>, String> {
        match self {
            Store::File(root) => {
                let path = root.join(provider);
                match std::fs::read(&path) {
                    Ok(bytes) => Ok(Some(bytes)),
                    // A file that is not there is a fact, not a failure.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(format!("{}: {e}", path.display())),
                }
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            Store::Platform => match Self::entry(provider)?.get_password() {
                Ok(password) => Ok(Some(password.into_bytes())),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(format!("the platform store refused: {e}")),
            },
        }
    }

    fn write(&self, provider: &str, secret: &[u8]) -> Result<(), String> {
        match self {
            Store::File(root) => {
                std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
                        .map_err(|e| format!("{}: {e}", root.display()))?;
                }
                // Through the same atomic seam as every other write tapkey makes, and created at
                // 0600 rather than corrected afterwards.
                crate::atomic::write_atomically(&root.join(provider), secret, 0o600)
                    .map_err(|e| format!("{e}"))
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            Store::Platform => Self::entry(provider)?
                .set_password(&String::from_utf8_lossy(secret))
                .map_err(|e| format!("the platform store refused: {e}")),
        }
    }

    fn remove(&self, provider: &str) -> Result<(), String> {
        match self {
            Store::File(root) => {
                let path = root.join(provider);
                match std::fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(format!("{}: {e}", path.display())),
                }
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            Store::Platform => Self::entry(provider)?
                .delete_credential()
                .map_err(|e| format!("the platform store refused: {e}")),
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl Store {
    fn entry(provider: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, provider)
            .map_err(|e| format!("the platform store refused: {e}"))
    }
}

/// Compatibility surface forever: renaming it later orphans every stored secret.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const SERVICE: &str = "tapkey";

fn default_file_root() -> PathBuf {
    // The same directory ADR-0019 gives the whole store, per platform. Reached only when the
    // file-backed kind is forced and no path was handed over — a real platform run has a
    // store directory from its caller, and Linux's own runs land here by default.
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        home.join("Library")
            .join("Application Support")
            .join("tapkey")
            .join("keys")
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        root.join("tapkey").join("keys")
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("tapkey"),
        None => PathBuf::from("."),
    }
}
