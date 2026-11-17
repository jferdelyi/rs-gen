//! Admin password and sessions.
//!
//! The server keeps only an Argon2id hash of the password, read from the file named by
//! `RSGEN_ADMIN_HASH_FILE`; a change of password writes the new hash to the same file.
//! A login opens a session: a random token kept in memory and sent back in an
//! `HttpOnly`, `Secure`, `SameSite=Strict` cookie.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::Rng;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Environment variable naming the file that holds the password hash.
pub(crate) const HASH_FILE_ENV: &str = "RSGEN_ADMIN_HASH_FILE";

/// Name of the session cookie.
pub(crate) const SESSION_COOKIE: &str = "rsgen_admin";

/// Lifetime of a session, from the login.
pub(crate) const SESSION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);

/// Logins without success from one address before it has to wait.
const MAX_ATTEMPTS: u32 = 5;

/// Wait imposed after `MAX_ATTEMPTS` logins without success.
const LOCKOUT: Duration = Duration::from_secs(60);

/// Attempts older than this are forgotten.
const ATTEMPT_MEMORY: Duration = Duration::from_secs(60 * 60);

/// Shortest password accepted by `hash-password`.
pub(crate) const MIN_PASSWORD_CHARS: usize = 12;

/// Longest password checked: hashing a huge input would only cost time.
pub(crate) const MAX_PASSWORD_BYTES: usize = 1024;

/// Logins without success of one client address.
struct Attempts {
	/// Attempts since the last wait.
	count: u32,

	/// Time of the last attempt.
	last: Instant,

	/// End of the current wait, if any.
	blocked_until: Option<Instant>,
}

/// Why a change of password failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChangeError {
	/// The current password is wrong.
	WrongPassword,

	/// The new password is refused (too short, too long, or the current one).
	Invalid(String),

	/// The new hash could not be saved: the password is unchanged.
	NotSaved(String),
}

/// Password hash, open sessions and login attempts.
pub(crate) struct Auth {
	/// Argon2id hash of the password, as a PHC string (`$argon2id$v=19$...`).
	password_hash: Mutex<String>,

	/// File the hash was read from, where a new hash is saved. `None`: the password cannot change.
	hash_file: Option<PathBuf>,

	/// Open sessions: token and end of validity.
	sessions: Mutex<HashMap<String, Instant>>,

	/// Logins without success by client address.
	attempts: Mutex<HashMap<String, Attempts>>,

	/// Held while a password is checked: one check at a time, each one taking about 19 MB.
	checking: Mutex<()>,

	/// Held during a change of password: one change at a time.
	changing: Mutex<()>,
}

impl Auth {
	/// Creates the authentication from a password hash. The password cannot be changed.
	///
	/// # Errors
	/// Returns an error if the hash is not a valid PHC string.
	pub(crate) fn new(password_hash: &str) -> Result<Self, String> {
		let password_hash = password_hash.trim();
		PasswordHash::new(password_hash).map_err(|e| format!("Invalid admin password hash: {}", e))?;
		Ok(Self {
			password_hash: Mutex::new(password_hash.to_owned()),
			hash_file: None,
			sessions: Mutex::new(HashMap::new()),
			attempts: Mutex::new(HashMap::new()),
			checking: Mutex::new(()),
			changing: Mutex::new(()),
		})
	}

	/// Reads the password hash from `path`, where a change of password saves the new hash.
	///
	/// # Errors
	/// Returns an error if the file cannot be read or holds no valid hash.
	pub(crate) fn from_file(path: &Path) -> Result<Self, String> {
		let hash = std::fs::read_to_string(path)
			.map_err(|e| format!("Cannot read {} ({}): {}", HASH_FILE_ENV, path.display(), e))?;
		let mut auth = Self::new(&hash)?;
		auth.hash_file = Some(path.to_owned());
		Ok(auth)
	}

	/// Reads the password hash from the file named by `RSGEN_ADMIN_HASH_FILE`.
	///
	/// # Returns
	/// - `Ok(None)` if the variable is not set: the admin routes stay disabled.
	/// - `Err` if the file cannot be read or holds no valid hash.
	pub(crate) fn from_env() -> Result<Option<Self>, String> {
		let Some(path) = std::env::var_os(HASH_FILE_ENV) else {
			return Ok(None);
		};
		Self::from_file(Path::new(&path)).map(Some)
	}

	/// Checks a password against the hash. Slow on purpose (Argon2id): call it off the async workers.
	pub(crate) fn verify(&self, password: &str) -> bool {
		if password.len() > MAX_PASSWORD_BYTES {
			return false;
		}
		let hash = lock(&self.password_hash).clone();
		let _checking = lock(&self.checking);
		Argon2::default().verify_password(password.as_bytes(), hash.as_str()).is_ok()
	}

	/// Changes the password: checks the current one, hashes the new one, saves the hash to
	/// the hash file, then uses it. If the hash cannot be saved, the password is unchanged.
	/// Slow on purpose (two Argon2id runs): call it off the async workers.
	///
	/// # Errors
	/// See `ChangeError`.
	pub(crate) fn change_password(&self, current: &str, new: &str) -> Result<(), ChangeError> {
		let _changing = lock(&self.changing);
		if !self.verify(current) {
			return Err(ChangeError::WrongPassword);
		}
		if new == current {
			return Err(ChangeError::Invalid("The new password must differ from the current one".to_owned()));
		}
		let hash = hash_password(new).map_err(ChangeError::Invalid)?;
		let Some(path) = &self.hash_file else {
			return Err(ChangeError::NotSaved(format!("The hash is not read from a file: set {}", HASH_FILE_ENV)));
		};
		save_hash(path, &hash).map_err(|e| ChangeError::NotSaved(format!("Cannot save the new hash to {}: {}", path.display(), e)))?;
		*lock(&self.password_hash) = hash;
		Ok(())
	}

	/// Records a login attempt of `client`, before its password is checked.
	///
	/// Counting the attempt first also counts the attempts sent at the same time.
	/// After `MAX_ATTEMPTS` attempts without success, the client has to wait.
	///
	/// # Returns
	/// - `Ok(())` if the client may try.
	/// - `Err(seconds)` if it has to wait that long first.
	pub(crate) fn begin_login(&self, client: &str) -> Result<(), u64> {
		let now = Instant::now();
		let mut attempts = lock(&self.attempts);
		// Forget the old attempts so that the table stays small
		attempts.retain(|_, entry| now.duration_since(entry.last) < ATTEMPT_MEMORY);
		let entry = attempts.entry(client.to_owned()).or_insert(Attempts { count: 0, last: now, blocked_until: None });
		if let Some(left) = entry.blocked_until.and_then(|end| end.checked_duration_since(now)) {
			return Err(left.as_secs().max(1));
		}
		entry.count += 1;
		entry.last = now;
		if entry.count >= MAX_ATTEMPTS {
			entry.count = 0;
			entry.blocked_until = Some(now + LOCKOUT);
		}
		Ok(())
	}

	/// Forgets the attempts of `client` after a successful login.
	pub(crate) fn record_success(&self, client: &str) {
		lock(&self.attempts).remove(client);
	}

	/// Opens a session and returns its token: 32 random bytes, in hexadecimal.
	pub(crate) fn open_session(&self) -> String {
		let mut bytes = [0u8; 32];
		rand::rng().fill(&mut bytes);
		let token: String = bytes.iter().map(|byte| format!("{:02x}", byte)).collect();
		let now = Instant::now();
		let mut sessions = lock(&self.sessions);
		sessions.retain(|_, end| *end > now);
		sessions.insert(token.clone(), now + SESSION_LIFETIME);
		token
	}

	/// Tells whether `token` belongs to an open session.
	pub(crate) fn has_session(&self, token: &str) -> bool {
		lock(&self.sessions).get(token).is_some_and(|end| *end > Instant::now())
	}

	/// Closes the session of `token`, if open.
	pub(crate) fn close_session(&self, token: &str) {
		lock(&self.sessions).remove(token);
	}

	/// Closes every session but the one of `kept`: after a change of password,
	/// whoever logged in with the old one is logged out.
	pub(crate) fn close_other_sessions(&self, kept: &str) {
		lock(&self.sessions).retain(|token, _| token == kept);
	}
}

/// Writes `hash` to `path` through a temporary file in the same directory, then renames
/// it: the file always holds a whole hash. Readable by its owner only, on Unix.
fn save_hash(path: &Path, hash: &str) -> std::io::Result<()> {
	let mut temporary = path.as_os_str().to_owned();
	temporary.push(".new");
	let temporary = PathBuf::from(temporary);
	let written = (|| {
		let mut options = std::fs::OpenOptions::new();
		options.write(true).create(true).truncate(true);
		#[cfg(unix)]
		std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
		let mut file = options.open(&temporary)?;
		file.write_all(hash.as_bytes())?;
		file.write_all(b"\n")?;
		file.sync_all()?;
		std::fs::rename(&temporary, path)
	})();
	if written.is_err() {
		let _ = std::fs::remove_file(&temporary);
	}
	written
}

/// Locks a table of `Auth`. A panic while it was locked cannot leave it half written
/// (each change is a single insert or remove), so a poisoned lock is used as is.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
	mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Hashes a password with Argon2id and a random salt, as a PHC string.
///
/// # Errors
/// Returns an error if the password is shorter than `MIN_PASSWORD_CHARS` characters
/// or longer than `MAX_PASSWORD_BYTES` bytes.
pub(crate) fn hash_password(password: &str) -> Result<String, String> {
	if password.chars().count() < MIN_PASSWORD_CHARS {
		return Err(format!("The password must have at least {} characters", MIN_PASSWORD_CHARS));
	}
	if password.len() > MAX_PASSWORD_BYTES {
		return Err(format!("The password must have at most {} bytes", MAX_PASSWORD_BYTES));
	}
	Argon2::default()
		.hash_password(password.as_bytes())
		.map(|hash| hash.to_string())
		.map_err(|e| format!("Cannot hash the password: {}", e))
}

/// `rs-gen-server hash-password`: reads a password on the standard input
/// and prints its hash, to be stored in the file named by `RSGEN_ADMIN_HASH_FILE`.
///
/// # Errors
/// Returns the message to show if the input cannot be read or the password is refused.
pub(crate) fn run_hash_password() -> Result<(), String> {
	let mut input = String::new();
	std::io::stdin().read_to_string(&mut input).map_err(|e| format!("Cannot read the password: {}", e))?;
	let password = input.strip_suffix('\n').unwrap_or(&input);
	let password = password.strip_suffix('\r').unwrap_or(password);
	println!("{}", hash_password(password)?);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	// Hashing costs about 20 MB and some tens of milliseconds: one hash for the whole module
	fn auth() -> Auth {
		static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
		Auth::new(HASH.get_or_init(|| hash_password("correct horse battery").unwrap())).unwrap()
	}

	#[test]
	fn hash_password_gives_an_argon2id_hash() {
		let hash = hash_password("correct horse battery").unwrap();
		assert!(hash.starts_with("$argon2id$"), "{}", hash);
		// A random salt: hashing twice gives two hashes
		assert_ne!(hash, hash_password("correct horse battery").unwrap());
	}

	#[test]
	fn hash_password_rejects_a_short_password() {
		assert!(hash_password("eleven char").is_err());
		assert!(hash_password(&"x".repeat(MAX_PASSWORD_BYTES + 1)).is_err());
	}

	#[test]
	fn verify_accepts_only_the_password() {
		let auth = auth();
		assert!(auth.verify("correct horse battery"));
		assert!(!auth.verify("correct horse batterY"));
		assert!(!auth.verify(""));
	}

	#[test]
	fn new_rejects_an_invalid_hash() {
		assert!(Auth::new("not a hash").is_err());
		assert!(Auth::new("").is_err());
	}

	#[test]
	fn sessions_open_and_close() {
		let auth = auth();
		let token = auth.open_session();
		assert_eq!(token.len(), 64);
		assert!(auth.has_session(&token));
		assert!(!auth.has_session("unknown"));
		assert_ne!(token, auth.open_session());
		auth.close_session(&token);
		assert!(!auth.has_session(&token));
	}

	#[test]
	fn an_expired_session_is_refused() {
		let auth = auth();
		let token = auth.open_session();
		lock(&auth.sessions).insert(token.clone(), Instant::now() - Duration::from_secs(1));
		assert!(!auth.has_session(&token));
	}

	#[test]
	fn too_many_attempts_make_the_client_wait() {
		let auth = auth();
		for _ in 0..MAX_ATTEMPTS {
			assert_eq!(auth.begin_login("1.2.3.4"), Ok(()));
		}
		let wait = auth.begin_login("1.2.3.4").unwrap_err();
		assert!((1..=LOCKOUT.as_secs()).contains(&wait), "{}", wait);
		// Another address may still try
		assert_eq!(auth.begin_login("5.6.7.8"), Ok(()));
	}

	// A hash file of "correct horse battery" dedicated to one test
	fn hash_file(test: &str) -> PathBuf {
		let path = std::env::temp_dir().join(format!("rs-gen-auth-{}-{}", std::process::id(), test));
		std::fs::write(&path, format!("{}\n", hash_password("correct horse battery").unwrap())).unwrap();
		path
	}

	#[test]
	fn change_password_saves_the_new_hash() {
		let path = hash_file("change");
		let auth = Auth::from_file(&path).unwrap();
		auth.change_password("correct horse battery", "a brand new password").unwrap();
		assert!(auth.verify("a brand new password"));
		assert!(!auth.verify("correct horse battery"));
		// The file holds the new hash: a restart keeps the new password
		assert!(Auth::from_file(&path).unwrap().verify("a brand new password"));
		assert!(!PathBuf::from(format!("{}.new", path.display())).exists());
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
		}
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn change_password_refuses_a_wrong_current_password_or_a_bad_new_one() {
		let path = hash_file("refuse");
		let before = std::fs::read_to_string(&path).unwrap();
		let auth = Auth::from_file(&path).unwrap();
		assert_eq!(auth.change_password("correct horse batterY", "a brand new password"), Err(ChangeError::WrongPassword));
		assert!(matches!(auth.change_password("correct horse battery", "too short"), Err(ChangeError::Invalid(_))));
		assert!(matches!(auth.change_password("correct horse battery", "correct horse battery"), Err(ChangeError::Invalid(_))));
		assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
		assert!(auth.verify("correct horse battery"));
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn a_hash_that_cannot_be_saved_keeps_the_password() {
		// No hash file
		let auth = auth();
		assert!(matches!(auth.change_password("correct horse battery", "a brand new password"), Err(ChangeError::NotSaved(_))));
		assert!(auth.verify("correct horse battery"));

		// The temporary file cannot be written: a directory has its name
		let path = hash_file("not-saved");
		let before = std::fs::read_to_string(&path).unwrap();
		let blocker = PathBuf::from(format!("{}.new", path.display()));
		std::fs::create_dir_all(&blocker).unwrap();
		let auth = Auth::from_file(&path).unwrap();
		assert!(matches!(auth.change_password("correct horse battery", "a brand new password"), Err(ChangeError::NotSaved(_))));
		assert!(auth.verify("correct horse battery"));
		assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
		let _ = std::fs::remove_dir(&blocker);
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn close_other_sessions_keeps_only_one() {
		let auth = auth();
		let kept = auth.open_session();
		let other = auth.open_session();
		auth.close_other_sessions(&kept);
		assert!(auth.has_session(&kept));
		assert!(!auth.has_session(&other));
	}

	#[test]
	fn a_success_forgets_the_attempts() {
		let auth = auth();
		for _ in 0..MAX_ATTEMPTS - 1 {
			auth.begin_login("1.2.3.4").unwrap();
		}
		auth.record_success("1.2.3.4");
		for _ in 0..MAX_ATTEMPTS {
			assert_eq!(auth.begin_login("1.2.3.4"), Ok(()));
		}
	}
}
