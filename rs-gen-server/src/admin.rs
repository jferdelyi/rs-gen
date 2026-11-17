//! Admin routes, under `/v1/admin`: the login, then the model management behind a session.
//!
//! Every admin request carries the `X-Rs-Gen-Admin` header, and every route but the login
//! needs an open session. The routes are disabled when no password hash is configured.

use crate::AppState;
use crate::auth::{SESSION_COOKIE, SESSION_LIFETIME};
use crate::catalog::{AdminError, ModelInfo};
use crate::check::{self, CheckReport, MAX_LINE_CHARS, MAX_UPLOAD_BYTES, MEMORY_MARGIN_BYTES};
use crate::system::MemoryInfo;
use actix_web::body::BoxBody;
use actix_web::cookie::{Cookie, SameSite};
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::http::StatusCode;
use actix_web::middleware::{Next, from_fn};
use actix_web::{HttpRequest, HttpResponse, web};
use serde::{Deserialize, Serialize};

/// Header that every admin request must carry. A page of another site cannot add it
/// without a CORS preflight, which the server refuses: cross-site requests are blocked.
pub(crate) const ADMIN_HEADER: &str = "x-rs-gen-admin";

/// Registers the admin routes.
pub(crate) fn configure(cfg: &mut web::ServiceConfig) {
	cfg.service(
		web::scope("/v1/admin")
			.wrap(from_fn(require_admin_header))
			.route("/login", web::post().to(login))
			.service(
				web::scope("")
					.wrap(from_fn(require_session))
					.app_data(web::PayloadConfig::new(MAX_UPLOAD_BYTES))
					.route("/logout", web::post().to(logout))
					.route("/session", web::get().to(session))
					.route("/models", web::get().to(list_models))
					.route("/models", web::post().to(add_model))
					.route("/models/check", web::post().to(check_model))
					.route("/models/{name}/hide", web::post().to(hide_model))
					.route("/models/{name}/show", web::post().to(show_model))
					.route("/models/{name}", web::delete().to(delete_model)),
			),
	);
}

/// Body of an error response.
#[derive(Serialize)]
struct ErrorBody<'a> {
	/// What went wrong.
	error: &'a str,
}

/// Body of a successful action.
#[derive(Serialize)]
struct Done {
	/// Always true.
	ok: bool,
}

/// Builds an error response.
fn error_response(status: StatusCode, message: &str) -> HttpResponse {
	HttpResponse::build(status).json(ErrorBody { error: message })
}

/// Builds the response of a failed admin action.
fn admin_error(error: &AdminError) -> HttpResponse {
	match error {
		AdminError::NotFound(message) => error_response(StatusCode::NOT_FOUND, message),
		AdminError::Conflict(message) => error_response(StatusCode::CONFLICT, message),
		AdminError::NotEnoughMemory(message) => error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
		AdminError::Internal(message) => error_response(StatusCode::INTERNAL_SERVER_ERROR, message),
	}
}

/// Refuses the admin requests when the admin is disabled or the admin header is missing.
async fn require_admin_header(req: ServiceRequest, next: Next<BoxBody>) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
	let enabled = req.app_data::<web::Data<AppState>>().is_some_and(|state| state.auth.is_some());
	if !enabled {
		return Ok(req.into_response(error_response(StatusCode::SERVICE_UNAVAILABLE, "The admin is disabled: no password hash is configured")));
	}
	if !req.headers().contains_key(ADMIN_HEADER) {
		return Ok(req.into_response(error_response(StatusCode::FORBIDDEN, "Missing admin header")));
	}
	next.call(req).await
}

/// Refuses the requests without an open session.
async fn require_session(req: ServiceRequest, next: Next<BoxBody>) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
	let valid = match (req.app_data::<web::Data<AppState>>(), req.cookie(SESSION_COOKIE)) {
		(Some(state), Some(cookie)) => state.auth.as_ref().is_some_and(|auth| auth.has_session(cookie.value())),
		_ => false,
	};
	if !valid {
		return Ok(req.into_response(error_response(StatusCode::UNAUTHORIZED, "Login required")));
	}
	next.call(req).await
}

/// Address of the client. The server listens on 127.0.0.1 only, behind nginx,
/// which overwrites `X-Real-IP` with the address the request came from.
fn client_address(req: &HttpRequest) -> String {
	req.headers()
		.get("x-real-ip")
		.and_then(|value| value.to_str().ok())
		.map(str::to_owned)
		.or_else(|| req.peer_addr().map(|address| address.ip().to_string()))
		.unwrap_or_else(|| "unknown".to_owned())
}

/// Session cookie. Without a `Path`, the browser sends it back to the admin routes only.
fn session_cookie(token: String) -> Cookie<'static> {
	Cookie::build(SESSION_COOKIE, token)
		.http_only(true)
		.secure(true)
		.same_site(SameSite::Strict)
		.max_age(actix_web::cookie::time::Duration::seconds(SESSION_LIFETIME.as_secs() as i64))
		.finish()
}

/// Body of `POST /v1/admin/login`.
#[derive(Deserialize)]
struct LoginBody {
	/// The admin password.
	password: String,
}

/// `POST /v1/admin/login`: checks the password and opens a session.
///
/// # Returns
/// - 200 OK with the session cookie
/// - 401 Unauthorized if the password is wrong
/// - 429 Too Many Requests after 5 attempts without success, for one minute
async fn login(req: HttpRequest, state: web::Data<AppState>, body: web::Json<LoginBody>) -> HttpResponse {
	let Some(auth) = state.auth.as_ref() else {
		return error_response(StatusCode::SERVICE_UNAVAILABLE, "The admin is disabled");
	};
	let client = client_address(&req);
	if let Err(wait) = auth.begin_login(&client) {
		return HttpResponse::TooManyRequests()
			.insert_header(("Retry-After", wait.to_string()))
			.json(ErrorBody { error: &format!("Too many failed logins: wait {} seconds", wait) });
	}

	let password = body.into_inner().password;
	let checker = state.clone();
	let valid = web::block(move || checker.auth.as_ref().is_some_and(|auth| auth.verify(&password)))
		.await
		.unwrap_or(false);
	if !valid {
		eprintln!("admin: failed login from {}", client);
		return error_response(StatusCode::UNAUTHORIZED, "Wrong password");
	}
	auth.record_success(&client);
	eprintln!("admin: login from {}", client);
	HttpResponse::Ok().cookie(session_cookie(auth.open_session())).json(Done { ok: true })
}

/// `POST /v1/admin/logout`: closes the session.
async fn logout(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
	if let (Some(auth), Some(cookie)) = (state.auth.as_ref(), req.cookie(SESSION_COOKIE)) {
		auth.close_session(cookie.value());
	}
	let mut removal = session_cookie(String::new());
	removal.make_removal();
	HttpResponse::Ok().cookie(removal).json(Done { ok: true })
}

/// `GET /v1/admin/session`: tells that the session is open (the middleware refuses it otherwise).
async fn session() -> HttpResponse {
	HttpResponse::Ok().json(Done { ok: true })
}

/// Limits of the server, shown on the admin page.
#[derive(Serialize)]
struct Limits {
	/// Largest text file accepted, in bytes.
	max_upload_bytes: usize,

	/// Longest line accepted, in characters.
	max_line_chars: usize,

	/// Memory kept free when adding or showing a model, in bytes.
	memory_margin_bytes: u64,
}

/// Body of `GET /v1/admin/models`.
#[derive(Serialize)]
struct ModelsResponse {
	/// Every model, sorted by name.
	models: Vec<ModelInfo>,

	/// Memory of the server.
	memory: MemoryInfo,

	/// Memory a new model can still take, in bytes, if known.
	available_bytes: Option<u64>,

	/// Limits of the server.
	limits: Limits,
}

/// `GET /v1/admin/models`: lists every model with its size, and the memory of the server.
async fn list_models(state: web::Data<AppState>) -> HttpResponse {
	match state.catalog.list() {
		Ok(models) => {
			let memory = MemoryInfo::read();
			HttpResponse::Ok().json(ModelsResponse {
				models,
				memory,
				available_bytes: memory.available_bytes(),
				limits: Limits { max_upload_bytes: MAX_UPLOAD_BYTES, max_line_chars: MAX_LINE_CHARS, memory_margin_bytes: MEMORY_MARGIN_BYTES },
			})
		}
		Err(error) => admin_error(&error),
	}
}

/// Query of the routes that receive a new model.
#[derive(Deserialize)]
struct NameQuery {
	/// Name of the new model.
	name: String,
}

/// Checks a text file for the model `name`.
fn check(state: &AppState, name: &str, bytes: &[u8]) -> CheckReport {
	let taken = check::is_valid_name(name) && state.catalog.name_taken(name);
	check::check_upload(name, taken, bytes, MemoryInfo::read().available_bytes())
}

/// `POST /v1/admin/models/check?name=<name>`, with the text file as body:
/// checks whether it can become a model, without saving anything.
async fn check_model(state: web::Data<AppState>, query: web::Query<NameQuery>, body: web::Bytes) -> HttpResponse {
	match web::block(move || check(&state, &query.name, &body)).await {
		Ok(report) => HttpResponse::Ok().json(report),
		Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "The check was interrupted"),
	}
}

/// Outcome of an addition.
enum Addition {
	/// The check passed and the build started.
	Started(CheckReport),
	/// The check failed.
	Refused(CheckReport),
	/// The model could not be reserved.
	Failed(AdminError),
}

/// `POST /v1/admin/models?name=<name>`, with the text file as body: checks it again,
/// then builds the model in the background. The model is listed as building, then
/// shown once it generates words, or listed as failed.
///
/// # Returns
/// - 202 Accepted with the check report: the build started
/// - 422 Unprocessable Content with the check report: the check failed
/// - 409 Conflict if the name is taken or another model is being built
async fn add_model(state: web::Data<AppState>, query: web::Query<NameQuery>, body: web::Bytes) -> HttpResponse {
	let addition = web::block(move || {
		let name = query.into_inner().name;
		let report = check(&state, &name, &body);
		if !report.ok {
			return Addition::Refused(report);
		}
		if let Err(error) = state.catalog.reserve(&name, &body) {
			return Addition::Failed(error);
		}
		let (builder, built) = (state.clone(), name.clone());
		let spawned = std::thread::Builder::new()
			.name("model-build".to_owned())
			.spawn(move || builder.catalog.build(&builder.generator, &built));
		if let Err(e) = spawned {
			state.catalog.abandon(&name, format!("Cannot start the build: {}", e));
		}
		eprintln!("admin: model '{}' added", name);
		Addition::Started(report)
	})
	.await;

	match addition {
		Ok(Addition::Started(report)) => HttpResponse::Accepted().json(report),
		Ok(Addition::Refused(report)) => HttpResponse::UnprocessableEntity().json(report),
		Ok(Addition::Failed(error)) => admin_error(&error),
		Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "The addition was interrupted"),
	}
}

/// Runs an action on the model `name` off the async workers (it may load or free a model).
async fn run_action<F>(state: web::Data<AppState>, name: String, verb: &'static str, action: F) -> HttpResponse
where
	F: FnOnce(&AppState, &str) -> Result<(), AdminError> + Send + 'static,
{
	if !check::is_valid_name(&name) {
		return error_response(StatusCode::BAD_REQUEST, "Invalid model name");
	}
	match web::block(move || action(&state, &name).map(|()| name)).await {
		Ok(Ok(name)) => {
			eprintln!("admin: model '{}' {}", name, verb);
			HttpResponse::Ok().json(Done { ok: true })
		}
		Ok(Err(error)) => admin_error(&error),
		Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "The action was interrupted"),
	}
}

/// `POST /v1/admin/models/{name}/hide`: unloads a model and moves its files to `data/hidden/`.
async fn hide_model(state: web::Data<AppState>, path: web::Path<String>) -> HttpResponse {
	run_action(state, path.into_inner(), "hidden", |state, name| state.catalog.hide(&state.generator, name)).await
}

/// `POST /v1/admin/models/{name}/show`: loads a hidden model again, if the memory allows it.
async fn show_model(state: web::Data<AppState>, path: web::Path<String>) -> HttpResponse {
	run_action(state, path.into_inner(), "shown", |state, name| {
		state.catalog.show(&state.generator, name, MemoryInfo::read().available_bytes())
	})
	.await
}

/// Body of `DELETE /v1/admin/models/{name}`.
#[derive(Deserialize)]
struct DeleteBody {
	/// The model name, typed again to confirm.
	confirm: String,
}

/// `DELETE /v1/admin/models/{name}`, with `{"confirm": "<name>"}`: deletes a model and its files for good.
async fn delete_model(state: web::Data<AppState>, path: web::Path<String>, body: web::Json<DeleteBody>) -> HttpResponse {
	let name = path.into_inner();
	if body.confirm != name {
		return error_response(StatusCode::BAD_REQUEST, "Type the model name to confirm the deletion");
	}
	run_action(state, name, "deleted", |state, name| state.catalog.delete(&state.generator, name)).await
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::auth::{Auth, hash_password};
	use actix_web::App;
	use actix_web::body::to_bytes;
	use actix_web::test::{self, TestRequest};
	use std::path::PathBuf;
	use std::time::{Duration, Instant};

	const PASSWORD: &str = "correct horse battery";

	#[derive(Serialize)]
	struct Password<'a> {
		password: &'a str,
	}

	#[derive(Serialize)]
	struct Confirm<'a> {
		confirm: &'a str,
	}

	#[derive(Deserialize)]
	struct Report {
		ok: bool,
	}

	#[derive(Deserialize)]
	struct Models {
		models: Vec<Model>,
	}

	#[derive(Deserialize)]
	struct Model {
		name: String,
		status: String,
	}

	// Hashing is slow on purpose: one hash for every test
	fn auth() -> Auth {
		static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
		Auth::new(HASH.get_or_init(|| hash_password(PASSWORD).unwrap())).unwrap()
	}

	// Creates a data directory dedicated to one test, and the state of a server reading it
	fn state(test: &str, models: &[(&str, &str)], with_auth: bool) -> (web::Data<AppState>, PathBuf) {
		let dir = std::env::temp_dir().join(format!("rs-gen-admin-{}-{}", std::process::id(), test));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		for (name, lines) in models {
			std::fs::write(dir.join(format!("{}.dat", name)), lines).unwrap();
		}
		let auth = if with_auth { Some(auth()) } else { None };
		(web::Data::new(AppState::new(&dir, auth).unwrap()), dir)
	}

	// Every route behind the session, as (method, path)
	const PROTECTED_ROUTES: &[(&str, &str)] = &[
		("POST", "/v1/admin/logout"),
		("GET", "/v1/admin/session"),
		("GET", "/v1/admin/models"),
		("POST", "/v1/admin/models?name=fruits"),
		("POST", "/v1/admin/models/check?name=fruits"),
		("POST", "/v1/admin/models/fruits/hide"),
		("POST", "/v1/admin/models/fruits/show"),
		("DELETE", "/v1/admin/models/fruits"),
		("GET", "/v1/admin/unknown"),
	];

	fn request(method: &str, path: &str) -> TestRequest {
		let request = match method {
			"GET" => TestRequest::get(),
			"POST" => TestRequest::post(),
			"DELETE" => TestRequest::delete(),
			_ => unreachable!(),
		};
		request.uri(path)
	}

	// An admin request, with the admin header
	fn admin(method: &str, path: &str) -> TestRequest {
		request(method, path).insert_header((ADMIN_HEADER, "1"))
	}

	// Logs in and returns the session cookie
	macro_rules! login_cookie {
		($app:expr) => {{
			let response = test::call_service($app, admin("POST", "/v1/admin/login").set_json(Password { password: PASSWORD }).to_request()).await;
			assert_eq!(response.status(), StatusCode::OK);
			response.response().cookies().find(|cookie| cookie.name() == SESSION_COOKIE).unwrap().into_owned()
		}};
	}

	// Waits for a model to leave the building status, and returns its status
	macro_rules! wait_build {
		($app:expr, $cookie:expr, $name:expr) => {{
			let start = Instant::now();
			loop {
				let response = test::call_service($app, admin("GET", "/v1/admin/models").cookie($cookie.clone()).to_request()).await;
				let body: Models = test::read_body_json(response).await;
				let status = body.models.into_iter().find(|model| model.name == $name).map(|model| model.status).unwrap_or_default();
				if status != "building" {
					break status;
				}
				assert!(start.elapsed() < Duration::from_secs(10), "the build did not end within 10 seconds");
				actix_web::rt::time::sleep(Duration::from_millis(20)).await;
			}
		}};
	}

	#[actix_web::test]
	async fn every_admin_route_needs_a_session() {
		let (state, dir) = state("needs-session", &[("fruits", "pomme\n")], true);
		let app = test::init_service(App::new().app_data(state).configure(crate::configure)).await;
		for (method, path) in PROTECTED_ROUTES {
			let response = test::call_service(&app, admin(method, path).to_request()).await;
			assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{} {}", method, path);
		}
		// A made-up session is refused too
		let forged = Cookie::new(SESSION_COOKIE, "0".repeat(64));
		let response = test::call_service(&app, admin("GET", "/v1/admin/models").cookie(forged).to_request()).await;
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[actix_web::test]
	async fn every_admin_route_needs_the_admin_header() {
		let (state, dir) = state("needs-header", &[("fruits", "pomme\n")], true);
		let app = test::init_service(App::new().app_data(state).configure(crate::configure)).await;
		let cookie = login_cookie!(&app);
		for (method, path) in PROTECTED_ROUTES.iter().chain(&[("POST", "/v1/admin/login")]) {
			let response = test::call_service(&app, request(method, path).cookie(cookie.clone()).to_request()).await;
			assert_eq!(response.status(), StatusCode::FORBIDDEN, "{} {}", method, path);
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[actix_web::test]
	async fn the_admin_is_disabled_without_a_password_hash() {
		let (state, dir) = state("disabled", &[("fruits", "pomme\n")], false);
		let app = test::init_service(App::new().app_data(state).configure(crate::configure)).await;
		let response = test::call_service(&app, admin("POST", "/v1/admin/login").set_json(Password { password: PASSWORD }).to_request()).await;
		assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
		let response = test::call_service(&app, admin("GET", "/v1/admin/models").to_request()).await;
		assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
		// The public routes still work
		let response = test::call_service(&app, TestRequest::get().uri("/v1/models").to_request()).await;
		assert_eq!(response.status(), StatusCode::OK);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[actix_web::test]
	async fn a_login_opens_a_session_until_the_logout() {
		let (state, dir) = state("session", &[("fruits", "pomme\n")], true);
		let app = test::init_service(App::new().app_data(state).configure(crate::configure)).await;
		let cookie = login_cookie!(&app);
		assert_eq!(cookie.http_only(), Some(true));
		assert_eq!(cookie.secure(), Some(true));
		assert_eq!(cookie.same_site(), Some(SameSite::Strict));
		assert_eq!(cookie.path(), None);

		let response = test::call_service(&app, admin("GET", "/v1/admin/session").cookie(cookie.clone()).to_request()).await;
		assert_eq!(response.status(), StatusCode::OK);
		let response = test::call_service(&app, admin("POST", "/v1/admin/logout").cookie(cookie.clone()).to_request()).await;
		assert_eq!(response.status(), StatusCode::OK);
		let response = test::call_service(&app, admin("GET", "/v1/admin/session").cookie(cookie).to_request()).await;
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[actix_web::test]
	async fn wrong_passwords_are_refused_then_slowed_down() {
		let (state, dir) = state("wrong-password", &[("fruits", "pomme\n")], true);
		let app = test::init_service(App::new().app_data(state).configure(crate::configure)).await;
		let attempt = |password: &str| {
			admin("POST", "/v1/admin/login")
				.insert_header(("x-real-ip", "203.0.113.7"))
				.set_json(Password { password })
				.to_request()
		};
		for _ in 0..5 {
			let response = test::call_service(&app, attempt("wrong password")).await;
			assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
			assert!(response.response().cookies().next().is_none());
		}
		// Even the right password waits
		let response = test::call_service(&app, attempt(PASSWORD)).await;
		assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
		assert!(response.headers().contains_key("retry-after"));
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[actix_web::test]
	async fn a_model_is_checked_added_hidden_shown_and_deleted() {
		let (state, dir) = state("lifecycle", &[("fruits", "pomme\npoire\n")], true);
		let app = test::init_service(App::new().app_data(state).configure(crate::configure)).await;
		let cookie = login_cookie!(&app);

		// Check: nothing is saved
		let response = test::call_service(&app, admin("POST", "/v1/admin/models/check?name=legumes").cookie(cookie.clone()).set_payload("chou\ncarotte\n").to_request()).await;
		assert_eq!(response.status(), StatusCode::OK);
		let report: Report = test::read_body_json(response).await;
		assert!(report.ok);
		assert!(!dir.join("legumes.dat").exists());

		// Add, then wait for the build
		let response = test::call_service(&app, admin("POST", "/v1/admin/models?name=legumes").cookie(cookie.clone()).set_payload("chou\ncarotte\n").to_request()).await;
		assert_eq!(response.status(), StatusCode::ACCEPTED);
		assert_eq!(wait_build!(&app, cookie, "legumes"), "shown");
		assert!(dir.join("legumes.dat").exists());

		// The same name again is refused, with the report
		let response = test::call_service(&app, admin("POST", "/v1/admin/models?name=legumes").cookie(cookie.clone()).set_payload("navet\n").to_request()).await;
		assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
		let report: Report = test::read_body_json(response).await;
		assert!(!report.ok);

		// Hide: gone from the public list
		let response = test::call_service(&app, admin("POST", "/v1/admin/models/legumes/hide").cookie(cookie.clone()).to_request()).await;
		assert_eq!(response.status(), StatusCode::OK);
		let public = to_bytes(test::call_service(&app, TestRequest::get().uri("/v1/models").to_request()).await.into_body()).await.unwrap();
		assert_eq!(public, "fruits");

		// Show again
		let response = test::call_service(&app, admin("POST", "/v1/admin/models/legumes/show").cookie(cookie.clone()).to_request()).await;
		assert_eq!(response.status(), StatusCode::OK);

		// Delete: the name must be typed again
		let response = test::call_service(&app, admin("DELETE", "/v1/admin/models/legumes").cookie(cookie.clone()).set_json(Confirm { confirm: "fruits" }).to_request()).await;
		assert_eq!(response.status(), StatusCode::BAD_REQUEST);
		let response = test::call_service(&app, admin("DELETE", "/v1/admin/models/legumes").cookie(cookie.clone()).set_json(Confirm { confirm: "legumes" }).to_request()).await;
		assert_eq!(response.status(), StatusCode::OK);
		assert!(!dir.join("legumes.dat").exists() && !dir.join("legumes.bin").exists());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[actix_web::test]
	async fn a_path_as_model_name_is_refused() {
		let (state, dir) = state("path-name", &[("fruits", "pomme\n")], true);
		let app = test::init_service(App::new().app_data(state).configure(crate::configure)).await;
		let cookie = login_cookie!(&app);
		let response = test::call_service(&app, admin("POST", "/v1/admin/models/..%2Ffruits/hide").cookie(cookie.clone()).to_request()).await;
		assert_eq!(response.status(), StatusCode::BAD_REQUEST);
		let response = test::call_service(&app, admin("POST", "/v1/admin/models?name=..%2Fevil").cookie(cookie).set_payload("x\n").to_request()).await;
		assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
		assert!(!dir.parent().unwrap().join("evil.dat").exists());
		assert!(dir.join("fruits.dat").exists());
		let _ = std::fs::remove_dir_all(&dir);
	}
}
