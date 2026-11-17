//! Admin password and sessions.
//!
//! The server keeps only an Argon2id hash of the password, read from the file named by
//! `RSGEN_ADMIN_HASH_FILE`. A login opens a session: a random token kept in memory and
//! sent back in an `HttpOnly`, `Secure`, `SameSite=Strict` cookie.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::Rng;
use std::collections::HashMap;
use std::io::Read;
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

/// Password hash, open sessions and login attempts.
pub(crate) struct Auth {
	/// Argon2id hash of the password, as a PHC string (`$argon2id$v=19$...`).
	password_hash: String,

	/// Open sessions: token and end of validity.
	sessions: Mutex<HashMap<String, Instant>>,

	/// Logins without success by client address.
	attempts: Mutex<HashMap<String, Attempts>>,

	/// Held while a password is checked: one check at a time, each one taking about 19 MB.
	checking: Mutex<()>,
}

impl Auth {
	/// Creates the authentication from a password hash.
	///
	/// # Errors
	/// Returns an error if the hash is not a valid PHC string.
	pub(crate) fn new(password_hash: &str) -> Result<Self, String> {
		let password_hash = password_hash.trim();
		PasswordHash::new(password_hash).map_err(|e| format!("Invalid admin password hash: {}", e))?;
		Ok(Self {
			password_hash: password_hash.to_owned(),
			sessions: Mutex::new(HashMap::new()),
			attempts: Mutex::new(HashMap::new()),
			checking: Mutex::new(()),
		})
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
		let hash = std::fs::read_to_string(&path)
			.map_err(|e| format!("Cannot read {} ({}): {}", HASH_FILE_ENV, path.to_string_lossy(), e))?;
		Self::new(&hash).map(Some)
	}

	/// Checks a password against the hash. Slow on purpose (Argon2id): call it off the async workers.
	pub(crate) fn verify(&self, password: &str) -> bool {
		if password.len() > MAX_PASSWORD_BYTES {
			return false;
		}
		let _checking = lock(&self.checking);
		Argon2::default().verify_password(password.as_bytes(), self.password_hash.as_str()).is_ok()
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
