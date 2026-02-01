use std::sync::Mutex;

use actix_web::{get, web, App, HttpResponse, HttpServer, Responder, http};
use actix_cors::Cors;

use rs_gen_core::model::generator::Generator;
use rs_gen_core::model::prediction_input::StartSeed;
use serde::Deserialize;

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

/// Shared application state for Actix, wrapping the generator in a Mutex
struct SharedData {
	model: Generator,
}

impl GenerateParams {
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
/// - 500 InternalServerError if the model is unavailable or mutex lock fails
#[get("/v1/generate")]
async fn get_generated(
	data: web::Data<Mutex<SharedData>>,
	query: web::Query<GenerateParams>,
) -> impl Responder {
	let max_n = query.max_n.unwrap_or(0);
	let nb_try = query.nb_try.unwrap_or(5);
	let randomness = query.randomness.unwrap_or(0.1);
	let reduce_random = query.reduce_random.unwrap_or(false);

	let start_seed = match query.start_seed() {
		Ok(s) => s,
		Err(e) => return HttpResponse::BadRequest().body(e),
	};

	let mut shared_data = match data.lock() {
		Ok(m) => m,
		Err(_) => return HttpResponse::InternalServerError().body("Model lock failed"),
	};

	// Prepare prediction input
	let mut input = shared_data.model.make_prediction_input();
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

	match shared_data.model.predict(&input) {
		Ok(result) => HttpResponse::Ok().body(result),
		Err(e) => HttpResponse::InternalServerError().body(e),
	}
}

/// HTTP GET `/v1/models` endpoint
///
/// Returns a newline-separated list of all loaded model names.
#[get("/v1/models")]
async fn get_models(data: web::Data<Mutex<SharedData>>) -> impl Responder {
	let shared_data = match data.lock() {
		Ok(m) => m,
		Err(_) => return HttpResponse::InternalServerError().body("Model lock failed"),
	};
	HttpResponse::Ok().body(shared_data.model.get_model_names().join("\n"))
}

/// Main entry point for the Actix web server
///
/// Loads the generator, wraps it in a mutex, and starts an HTTP server.
///
/// # Notes
/// - Binds to 127.0.0.1:5000
/// - Currently, a model path is hardcoded; should be configurable in the future
/// - Handles concurrency via `Mutex`
#[actix_web::main]
async fn main() -> std::io::Result<()> {
	let generator = match Generator::new("./data") {
		Ok(g) => g,
		Err(e) => panic!("Failed to load model: {}", e),
	};

	let shared_data = SharedData { model: generator };
	let shared_model = web::Data::new(Mutex::new(shared_data));

	HttpServer::new(move || {
		App::new()
			.wrap(
				Cors::default()
					.allow_any_origin()
					.allowed_methods(vec!["GET", "POST", "OPTIONS"])
					.allowed_header(http::header::CONTENT_TYPE)
			)
			.app_data(shared_model.clone())
			.service(get_generated)
			.service(get_models)
	})
		.bind(("127.0.0.1", 5000))?
		.run()
		.await
}
