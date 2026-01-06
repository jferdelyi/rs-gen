use std::sync::Mutex;

use actix_web::{get, put, web, App, HttpResponse, HttpServer, Responder};

use serde::Deserialize;
use rs_gen_core::io::list_files;
use rs_gen_core::model::multigram_model::{MultiGramModel, StartSeed};

/// Struct representing query parameters for the `/v1/generate` endpoint
#[derive(Deserialize)]
struct GenerateParams {
	max_n: Option<usize>,
	nb_try: Option<usize>,
	randomness: Option<f32>,
	reduce_random: Option<bool>,
	seed: Option<String> // -> random(n) if 0 random ngram too, custom(str) or none
}

#[derive(Deserialize)]
struct ModelQuery {
	names: Option<String>
}

struct SharedData {
	model: MultiGramModel
}

impl GenerateParams {
	/// Determines the starting seed strategy for sequence generation.
	fn start_seed(&self) -> Result<StartSeed<'_>, String> {
		match &self.seed {
			None => Ok(StartSeed::False),
			Some(s) if s.to_lowercase() == "none" => Ok(StartSeed::False),
			Some(s) if s.to_lowercase().starts_with("custom:") => {
				let value = &s["custom:".len()..];
				if value.is_empty() {
					Err("Custom seed cannot be empty".into())
				} else {
					Ok(StartSeed::Custom(value))
				}
			}
			Some(s) if s.to_lowercase().starts_with("random:") => {
				let value = &s["random:".len()..];
				match value.parse::<usize>() {
					Ok(n) => Ok(StartSeed::Random(n)),
					Err(_) => Err("Random seed must be an integer".into()),
				}
			}
			Some(_) => Err("Seed must start with 'custom:' or 'random:' or be 'none'".into()),
		}
	}
}

/// HTTP GET endpoint `/v1/generate`
///
/// Generates a sequence using the GlobalNGramModel based on query parameters.
/// Returns a generated sequence as the response body.
#[get("/v1/generate")]
async fn get_generated(data: web::Data<Mutex<SharedData>>, query: web::Query<GenerateParams>) -> impl Responder {
	let max_n = query.max_n.unwrap_or(0);
	let nb_try = query.nb_try.unwrap_or(5);
	let randomness = query.randomness.unwrap_or(0.1);
	let reduce_random = query.reduce_random.unwrap_or(false);

	let start_seed = match query.start_seed() {
		Ok(s) => s,
		Err(e) => return HttpResponse::BadRequest().body(e)
	};

	let mut shared_data = match data.lock() {
		Ok(m) => m,
		Err(_) => return HttpResponse::InternalServerError().body("Model lock failed"),
	};

	match shared_data.model.predict(max_n, nb_try, randomness, reduce_random, &start_seed) {
		Ok(result) => HttpResponse::Ok().body(result),
		Err(e) => HttpResponse::InternalServerError().body(e),
	}
}

#[get("/v1/models")]
async fn get_models() -> impl Responder {
	match list_files(&"./data".to_owned(), "dat") {
		Ok(files) => HttpResponse::Ok().body(files.join("\n").replace(".dat", "")),
		Err(_) => HttpResponse::InternalServerError().body("Failed to list models")
	}
}

#[get("/v1/loaded_models")]
async fn get_loaded_models(data: web::Data<Mutex<SharedData>>) -> impl Responder {
	let shared_data = match data.lock() {
		Ok(m) => m,
		Err(_) => return HttpResponse::InternalServerError().body("Model lock failed"),
	};
	HttpResponse::Ok().body(shared_data.model.get_model_names().join("\n"))
}

#[put("/v1/load_models")]
async fn put_model(data: web::Data<Mutex<SharedData>>, query: web::Query<ModelQuery>) -> impl Responder {
	let mut shared_data = match data.lock() {
		Ok(m) => m,
		Err(_) => return HttpResponse::InternalServerError().body("Model lock failed"),
	};

	let query_names = match &query.names {
		Some(s) if !s.trim().is_empty() => s.trim(),
		_ => return HttpResponse::BadRequest().body("Missing or empty model name"),
	};

	let model_names: Vec<&str> = query_names
		.split(',')
		.map(|s| s.trim())
		.filter(|s| !s.is_empty())
		.collect();

	shared_data.model = MultiGramModel::default();
	for name in model_names {
		let model_path = format!("./data/{}.dat", name);
		let partial_model = match MultiGramModel::new(model_path) {
			Ok(m) => m,
			Err(e) => return HttpResponse::InternalServerError().body(format!("Failed to load model: {e}"))
		};
		match shared_data.model.merge(&partial_model) {
			Ok(_) => (),
			Err(e) => return HttpResponse::InternalServerError().body(format!("Failed to merge model: {e}"))
		}
	}

	HttpResponse::Ok().body("Models loaded successfully")
}

/// Main entry point for the server.
///
/// Loads the n-gram model, wraps it in a `Mutex` for thread safety,
/// and starts an Actix-web HTTP server with a single endpoint.
///
/// # Notes
/// - The server binds to 127.0.0.1:5000.
/// - Currently, the model file path is hardcoded and should be made configurable.
/// - WIP: Additional endpoints, error handling, and logging may be added.
#[actix_web::main]
async fn main() -> std::io::Result<()> {
	let shared_data = SharedData {
		model: MultiGramModel::default(),
	};
	let shared_model = web::Data::new(Mutex::new(shared_data));

	HttpServer::new(move || {
		App::new()
			.app_data(shared_model.clone())
			.service(get_generated)
			.service(get_models)
			.service(put_model)
			.service(get_loaded_models)
	})
		.bind(("127.0.0.1", 5000))?
		.run()
		.await
}
