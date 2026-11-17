mod admin;
mod auth;
mod catalog;
mod check;
mod system;

use std::path::Path;
use std::sync::RwLock;

use actix_web::{get, web, App, HttpResponse, HttpServer, Responder, http};
use actix_cors::Cors;

use auth::Auth;
use catalog::Catalog;
use system::MemoryInfo;
use rs_gen_core::model::generator::Generator;
use rs_gen_core::model::prediction_input::StartSeed;
use serde::Deserialize;

/// Highest number of attempts to avoid duplicates (the web front sends at most 100).
const MAX_NB_TRY: usize = 1000;

/// Query parameters for the `/v1/generate` endpoint
#[derive(Deserialize)]
struct GenerateParams {
	/// Maximum n-gram size to use (optional; default 0 = auto)
	max_n: Option<usize>,
	/// Number of attempts to avoid duplicates (optional; default 5)
	nb_try: Option<usize>,
	/// Randomness factor [0.0, 1.0] for choosing shorter n-grams
	randomness: Option<f32>,
	/// Whether to reduce randomness progressively
	reduce_random: Option<bool>,
	/// Seed string controlling the starting prefix
	/// Formats: "none", "custom:<string>", "random:<n-gram>"
	seed: Option<String>,
	/// Per-model intensity weights, format: "name1:0.5,name2:0.25"
	intensity: Option<String>,
}

/// Shared application state for Actix
pub(crate) struct AppState {
	/// Loaded models: read by the generation, changed by the admin routes
	pub(crate) generator: RwLock<Generator>,
	/// Model files of the data directory and their status
	pub(crate) catalog: Catalog,
	/// Admin password and sessions, `None` when the admin routes are disabled
	pub(crate) auth: Option<Auth>,
	/// Reads the memory of the server: `MemoryInfo::read`, replaced by fixed figures in tests
	pub(crate) memory: fn() -> MemoryInfo,
}

impl AppState {
	/// Loads the models of `data_dir` and creates the state of the server.
	///
	/// # Errors
	/// Returns an error if the directory or a model cannot be read.
	pub(crate) fn new<P: AsRef<Path>>(data_dir: P, auth: Option<Auth>) -> Result<Self, Box<dyn std::error::Error>> {
		let generator = Generator::new(&data_dir)?;
		let catalog = Catalog::new(&data_dir, &generator);
		Ok(Self { generator: RwLock::new(generator), catalog, auth, memory: MemoryInfo::read })
	}
}

impl GenerateParams {
	/// Returns the number of attempts to avoid duplicates (default 5).
	///
	/// # Errors
	/// Returns an error above `MAX_NB_TRY`: each attempt generates a word, so a huge
	/// value would keep the server busy for minutes.
	fn nb_try(&self) -> Result<usize, String> {
		match self.nb_try {
			None => Ok(5),
			Some(n) if n <= MAX_NB_TRY => Ok(n),
			Some(n) => Err(format!("nb_try must be at most {}, got {}", MAX_NB_TRY, n)),
		}
	}

	/// Computes the starting seed strategy for sequence generation
	///
	/// # Returns
	/// - `StartSeed::False` if no seed is specified or "none"
	/// - `StartSeed::Custom(s)` if "custom:<s>" is provided
	/// - `StartSeed::Random(n)` if "random:<n>" is provided
	///
	/// # Errors
	/// Returns a `String` describing invalid formats or values
	fn start_seed(&self) -> Result<StartSeed, String> {
		let seed : String = match &self.seed  {
			None => return Ok(StartSeed::False),
			Some(s) => s.to_lowercase()
		};

		match seed {
			s if s == "none" => Ok(StartSeed::False),
			s if s.starts_with("custom:") => {
				let value = &s["custom:".len()..];
				if value.is_empty() {
					return Err("Custom seed cannot be empty".into());
				}
				match value.parse::<String>() {
					Err(_) => Err("Custom seed must be a valid UTF-8 string".into()),
					Ok(_) => Ok(StartSeed::Custom(value.to_owned()))
				}
			}
			s if s.starts_with("random:") => {
				let value = &s["random:".len()..];
				if value.is_empty() {
					return Err("Random seed cannot be empty".into());
				}
				match value.parse::<usize>() {
					Ok(n) => Ok(StartSeed::Random(n)),
					Err(_) => Err("Random seed must be an integer".into()),
				}
			}
			_ => Err("Seed must start with 'custom:', 'random:', or be 'none'".into()),
		}
	}
}

/// HTTP GET `/v1/generate` endpoint
///
/// Generates a sequence from loaded models based on query parameters.
/// Handles randomness, number of attempts, seed selection, and per-model intensity.
///
/// # Query Parameters
/// - See `GenerateParams` struct
///
/// # Returns
/// - 200 OK with generated sequence
/// - 400 BadRequest if parameters are invalid
/// - 500 InternalServerError if the model is unavailable or the lock fails
#[get("/v1/generate")]
async fn get_generated(
	data: web::Data<AppState>,
	query: web::Query<GenerateParams>,
) -> impl Responder {
	let max_n = query.max_n.unwrap_or(0);
	let nb_try = match query.nb_try() {
		Ok(n) => n,
		Err(e) => return HttpResponse::BadRequest().body(e),
	};
	let randomness = query.randomness.unwrap_or(0.1);
	let reduce_random = query.reduce_random.unwrap_or(false);

	let start_seed = match query.start_seed() {
		Ok(s) => s,
		Err(e) => return HttpResponse::BadRequest().body(e),
	};

	let generator = match data.generator.read() {
		Ok(g) => g,
		Err(_) => return HttpResponse::InternalServerError().body("Model lock failed"),
	};

	// Prepare prediction input
	let mut input = generator.make_prediction_input();
	input.max_n = max_n;
	input.nb_try = nb_try;
	if let Err(e) = input.set_randomness(randomness) {
		return HttpResponse::BadRequest().body(e);
	}
	input.reduce_random = reduce_random;
	input.start_seed = start_seed;

	// Parse intensity query like "name1:0.5,name2:0.25"
	if let Some(intensity_str) = &query.intensity {
		for data in intensity_str.split(',') {
			let split: Vec<&str> = data.split(':').collect();
			if split.len() != 2 {
				return HttpResponse::BadRequest().body(format!("Invalid intensity format: {}", data));
			}
			let model_name = split[0];
			let intensity = match split[1].parse::<f32>() {
				Ok(i) => i,
				Err(_) => return HttpResponse::BadRequest().body(format!("Invalid intensity value: {}", split[1])),
			};
			if let Err(e) = input.set_model_intensity(model_name, intensity) {
				return HttpResponse::BadRequest().body(e);
			}
		}
	}

	match generator.predict(&input) {
		Ok(result) => HttpResponse::Ok().body(result),
		Err(e) => HttpResponse::InternalServerError().body(e),
	}
}

/// HTTP GET `/v1/models` endpoint
///
/// Returns a newline-separated list of the loaded model names (the hidden ones are not loaded).
#[get("/v1/models")]
async fn get_models(data: web::Data<AppState>) -> impl Responder {
	let generator = match data.generator.read() {
		Ok(g) => g,
		Err(_) => return HttpResponse::InternalServerError().body("Model lock failed"),
	};
	HttpResponse::Ok().body(generator.get_model_names().join("\n"))
}

/// Registers every route: the public ones and the admin ones.
pub(crate) fn configure(cfg: &mut web::ServiceConfig) {
	cfg.service(get_generated).service(get_models).configure(admin::configure);
}

/// Main entry point for the Actix web server
///
/// Loads the models, then starts an HTTP server.
/// `rs-gen-server hash-password` reads a password on the standard input and prints its hash instead.
///
/// # Notes
/// - Binds to 127.0.0.1:5000
/// - Currently, the model path is hardcoded (`./data`); should be configurable in the future
/// - The admin routes are enabled when `RSGEN_ADMIN_HASH_FILE` names a password hash file
#[actix_web::main]
async fn main() -> std::io::Result<()> {
	if std::env::args().nth(1).as_deref() == Some("hash-password") {
		if let Err(e) = auth::run_hash_password() {
			eprintln!("{}", e);
			std::process::exit(1);
		}
		return Ok(());
	}

	// A missing or invalid hash disables the admin routes only: the public routes keep running
	let auth = match Auth::from_env() {
		Ok(Some(a)) => Some(a),
		Ok(None) => {
			eprintln!("admin: disabled, {} is not set", auth::HASH_FILE_ENV);
			None
		}
		Err(e) => {
			eprintln!("admin: disabled, {}", e);
			None
		}
	};
	let state = match AppState::new("./data", auth) {
		Ok(s) => web::Data::new(s),
		Err(e) => panic!("Failed to load model: {}", e),
	};

	HttpServer::new(move || {
		App::new()
			.wrap(
				Cors::default()
					.allow_any_origin()
					.allowed_methods(vec!["GET", "POST", "OPTIONS"])
					.allowed_header(http::header::CONTENT_TYPE)
			)
			.app_data(state.clone())
			.configure(configure)
	})
		.bind(("127.0.0.1", 5000))?
		.run()
		.await
}

#[cfg(test)]
mod tests {
	use super::*;
	use actix_web::http::StatusCode;
	use actix_web::test::{self, TestRequest};

	fn params(nb_try: Option<usize>) -> GenerateParams {
		GenerateParams { max_n: None, nb_try, randomness: None, reduce_random: None, seed: None, intensity: None }
	}

	#[test]
	fn nb_try_is_bounded() {
		assert_eq!(params(None).nb_try(), Ok(5));
		assert_eq!(params(Some(MAX_NB_TRY)).nb_try(), Ok(MAX_NB_TRY));
		assert!(params(Some(MAX_NB_TRY + 1)).nb_try().is_err());
	}

	#[actix_web::test]
	async fn a_huge_nb_try_is_refused_at_once() {
		// The only word of the model always exists: every attempt fails, up to nb_try
		let dir = std::env::temp_dir().join(format!("rs-gen-server-{}-nb-try", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("low.dat"), "abc\n").unwrap();
		let state = web::Data::new(AppState::new(&dir, None).unwrap());
		let app = test::init_service(App::new().app_data(state).configure(configure)).await;

		let response = test::call_service(&app, TestRequest::get().uri("/v1/generate?nb_try=1000000000&intensity=low:1").to_request()).await;
		assert_eq!(response.status(), StatusCode::BAD_REQUEST);
		let response = test::call_service(&app, TestRequest::get().uri("/v1/generate?nb_try=1000&intensity=low:1").to_request()).await;
		assert_eq!(response.status(), StatusCode::OK);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
