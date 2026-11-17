//! Model files of the data directory and their status: shown, hidden, being built.
//!
//! - A shown model is `data/<name>.dat` (and its cache `data/<name>.bin`), loaded in the generator.
//! - A hidden model is moved to `data/hidden/`: it is not loaded and takes no memory.
//! - A model being added is built in `data/building/`, then moved to `data/` once it works.
//!
//! `Generator::new` reads only the files directly in `data/`: the sub-directories are ignored.

use crate::check::{Fit, MEMORY_MARGIN_BYTES, fit, megabytes, megabytes_or_unknown};
use crate::system::Room;
use rs_gen_core::model::generator::Generator;
use rs_gen_core::model::multigram_model::{ModelStats, MultiGramModel};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockWriteGuard};
use std::time::UNIX_EPOCH;

/// Words generated to check that a new model works.
const SMOKE_TEST_WORDS: usize = 5;

/// Why an admin action failed, with the HTTP status that goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdminError {
	/// No model has this name.
	NotFound(String),

	/// The model is not in a state that allows the action.
	Conflict(String),

	/// The server does not have the memory for the action, even with swap.
	NotEnoughMemory(String),

	/// The server has the memory for the action only by using swap: allowed when forced.
	MemoryRisk(String),

	/// Reading or writing a file failed.
	Internal(String),
}

impl From<std::io::Error> for AdminError {
	fn from(error: std::io::Error) -> Self {
		AdminError::Internal(error.to_string())
	}
}

/// Status of a model.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
	/// Loaded and used by the generation.
	Shown,
	/// Kept on disk, not loaded.
	Hidden,
	/// Being built from its text file.
	Building,
	/// Its build failed; it can be deleted.
	Failed,
}

/// Description of a model, sent to the admin page.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub(crate) struct ModelInfo {
	/// Name of the model.
	pub(crate) name: String,

	/// Status of the model.
	pub(crate) status: Status,

	/// Why the build failed, for a failed model.
	pub(crate) error: Option<String>,

	/// Size of the model, if it was loaded since the server started.
	pub(crate) stats: Option<ModelStats>,

	/// Number of transitions, if the model was loaded since the server started.
	pub(crate) transitions: Option<usize>,

	/// Estimated memory of the model once loaded, in bytes.
	pub(crate) estimated_bytes: Option<u64>,

	/// Size of the text file, in bytes.
	pub(crate) dat_bytes: Option<u64>,

	/// Size of the binary cache, in bytes.
	pub(crate) bin_bytes: Option<u64>,

	/// Last change of the text file, in seconds since 1970.
	pub(crate) modified: Option<u64>,
}

/// Size of a model, known once it was loaded.
#[derive(Debug, Clone, Copy)]
struct Known {
	/// Sentences, states and their bytes.
	stats: ModelStats,

	/// Number of transitions.
	transitions: usize,
}

impl Known {
	/// Reads the size of a loaded model.
	fn of(model: &MultiGramModel) -> Self {
		Self { stats: model.stats(), transitions: model.transition_count() }
	}
}

/// State of a model being added.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Job {
	/// Being built.
	Building,
	/// The build failed, for this reason.
	Failed(String),
}

/// Model files of the data directory and what is known about them.
pub(crate) struct Catalog {
	/// The data directory, read by `Generator::new`.
	data_dir: PathBuf,

	/// Size of the models loaded since the server started, shown or hidden since.
	known: Mutex<HashMap<String, Known>>,

	/// Models being added, or whose addition failed.
	jobs: Mutex<HashMap<String, Job>>,
}

impl Catalog {
	/// Creates the catalog of `data_dir`, whose shown models are already loaded in `generator`.
	pub(crate) fn new<P: AsRef<Path>>(data_dir: P, generator: &Generator) -> Self {
		let known = generator
			.get_model_names()
			.into_iter()
			.filter_map(|name| generator.get_model(&name).map(|model| (name, Known::of(model))))
			.collect();
		Self {
			data_dir: data_dir.as_ref().to_path_buf(),
			known: Mutex::new(known),
			jobs: Mutex::new(HashMap::new()),
		}
	}

	/// Directory of the hidden models.
	fn hidden_dir(&self) -> PathBuf {
		self.data_dir.join("hidden")
	}

	/// Directory of the models being built.
	fn building_dir(&self) -> PathBuf {
		self.data_dir.join("building")
	}

	/// Text file of the model `name` in `dir`.
	fn dat(dir: &Path, name: &str) -> PathBuf {
		dir.join(format!("{}.dat", name))
	}

	/// Binary cache of the model `name` in `dir`.
	fn bin(dir: &Path, name: &str) -> PathBuf {
		dir.join(format!("{}.bin", name))
	}

	/// Lists every model: shown, hidden, being built or failed, sorted by name.
	pub(crate) fn list(&self) -> Result<Vec<ModelInfo>, AdminError> {
		// Copied, not held: the other actions lock `jobs` before `known`
		let known = lock(&self.known).clone();
		let mut models = Vec::new();
		for (dir, status) in [(self.data_dir.clone(), Status::Shown), (self.hidden_dir(), Status::Hidden)] {
			for name in dat_names(&dir)? {
				let size = known.get(&name).copied();
				let dat = std::fs::metadata(Self::dat(&dir, &name)).ok();
				models.push(ModelInfo {
					status,
					error: None,
					stats: size.map(|size| size.stats),
					transitions: size.map(|size| size.transitions),
					estimated_bytes: size.map(|size| size.stats.estimated_bytes()),
					dat_bytes: dat.as_ref().map(|metadata| metadata.len()),
					bin_bytes: std::fs::metadata(Self::bin(&dir, &name)).ok().map(|metadata| metadata.len()),
					modified: dat
						.and_then(|metadata| metadata.modified().ok())
						.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
						.map(|duration| duration.as_secs()),
					name,
				});
			}
		}
		for (name, job) in lock(&self.jobs).iter() {
			let (status, error) = match job {
				Job::Building => (Status::Building, None),
				Job::Failed(error) => (Status::Failed, Some(error.clone())),
			};
			models.push(ModelInfo {
				name: name.clone(),
				status,
				error,
				stats: None,
				transitions: None,
				estimated_bytes: None,
				dat_bytes: std::fs::metadata(Self::dat(&self.building_dir(), name)).ok().map(|metadata| metadata.len()),
				bin_bytes: None,
				modified: None,
			});
		}
		models.sort_by(|a, b| a.name.cmp(&b.name));
		Ok(models)
	}

	/// Tells whether a model, shown, hidden or being added, already has this name.
	pub(crate) fn name_taken(&self, name: &str) -> bool {
		lock(&self.jobs).contains_key(name) || self.on_disk(name)
	}

	/// Tells whether the model is shown or hidden.
	fn on_disk(&self, name: &str) -> bool {
		Self::dat(&self.data_dir, name).exists() || Self::dat(&self.hidden_dir(), name).exists()
	}

	/// Hides a shown model: its files go to `data/hidden/` and it leaves the memory.
	pub(crate) fn hide(&self, generator: &RwLock<Generator>, name: &str) -> Result<(), AdminError> {
		let _jobs = lock(&self.jobs);
		if !Self::dat(&self.data_dir, name).exists() {
			return Err(self.not_shown(name));
		}
		std::fs::create_dir_all(self.hidden_dir())?;
		move_model(&self.data_dir, &self.hidden_dir(), name)?;
		let model = write(generator)?.remove_model(name);
		// Freed once the generator is unlocked: freeing a big model takes time
		drop(model);
		Ok(())
	}

	/// Error for an action on a model that should be shown.
	fn not_shown(&self, name: &str) -> AdminError {
		if Self::dat(&self.hidden_dir(), name).exists() {
			AdminError::Conflict(format!("The model '{}' is already hidden", name))
		} else {
			AdminError::NotFound(format!("No model named '{}'", name))
		}
	}

	/// Shows a hidden model again: its files come back to `data/` and it is loaded,
	/// if the server has the memory for it.
	///
	/// # Parameters
	/// - `room`: memory the server can still give, in RAM and with swap.
	/// - `force`: loads the model even if it fits only by using swap.
	pub(crate) fn show(&self, generator: &RwLock<Generator>, name: &str, room: Room, force: bool) -> Result<(), AdminError> {
		let _jobs = lock(&self.jobs);
		let hidden = self.hidden_dir();
		if !Self::dat(&hidden, name).exists() {
			return Err(if Self::dat(&self.data_dir, name).exists() {
				AdminError::Conflict(format!("The model '{}' is already shown", name))
			} else {
				AdminError::NotFound(format!("No model named '{}'", name))
			});
		}
		if Self::dat(&self.data_dir, name).exists() {
			return Err(AdminError::Conflict(format!("A shown model is already named '{}'", name)));
		}

		if room != Room::default() {
			let needed = self.needed_to_load(&hidden, name)?;
			match fit(needed, room) {
				Fit::InRam | Fit::Unknown => (),
				Fit::WithSwap if force => (),
				Fit::WithSwap => return Err(AdminError::MemoryRisk(format!(
					"Not enough RAM: loading '{}' needs about {}, the server has {} of RAM left (with a margin of {}). It fits by using swap, {} left with it: the generation will be slower",
					name, megabytes(needed), megabytes_or_unknown(room.ram_bytes), megabytes(MEMORY_MARGIN_BYTES), megabytes_or_unknown(room.with_swap_bytes)
				))),
				Fit::TooBig => return Err(AdminError::NotEnoughMemory(match room.with_swap_bytes {
					Some(with_swap) => format!(
						"Not enough memory, even with swap: loading '{}' needs about {}, the server has {} left with swap (with a margin of {})",
						name, megabytes(needed), megabytes(with_swap), megabytes(MEMORY_MARGIN_BYTES)
					),
					None => format!(
						"Not enough memory: loading '{}' needs about {}, the server has {} left (with a margin of {})",
						name, megabytes(needed), megabytes_or_unknown(room.ram_bytes), megabytes(MEMORY_MARGIN_BYTES)
					),
				})),
			}
		}

		move_model(&hidden, &self.data_dir, name)?;
		let model = match MultiGramModel::new(Self::dat(&self.data_dir, name)) {
			Ok(model) => model,
			Err(e) => {
				move_model(&self.data_dir, &hidden, name)?;
				return Err(AdminError::Internal(format!("Cannot load '{}': {}", name, e)));
			}
		};
		lock(&self.known).insert(name.to_owned(), Known::of(&model));
		write(generator)?.insert_model(model).map_err(AdminError::Conflict)
	}

	/// Estimated peak memory to load the model `name` of `dir`.
	///
	/// Its size is known if it was loaded since the server started, counted from its text file otherwise.
	/// Without a binary cache, the model is built: the peak is the one of a build.
	fn needed_to_load(&self, dir: &Path, name: &str) -> Result<u64, AdminError> {
		let dat_path = Self::dat(dir, name);
		let stats = match lock(&self.known).get(name) {
			Some(known) => known.stats,
			None => MultiGramModel::count_text(&std::fs::read_to_string(&dat_path)?, usize::MAX).stats,
		};
		let bin_bytes = std::fs::metadata(Self::bin(dir, name)).map(|metadata| metadata.len());
		Ok(match bin_bytes {
			// The cache is read whole before being decoded
			Ok(bin_bytes) => stats.estimated_bytes() + bin_bytes,
			Err(_) => stats.estimated_build_bytes(std::fs::metadata(&dat_path)?.len()),
		})
	}

	/// Deletes a model for good, shown, hidden or failed: its files are removed.
	pub(crate) fn delete(&self, generator: &RwLock<Generator>, name: &str) -> Result<(), AdminError> {
		let mut jobs = lock(&self.jobs);
		match jobs.get(name) {
			Some(Job::Building) => return Err(AdminError::Conflict(format!("The model '{}' is being built", name))),
			Some(Job::Failed(_)) => {
				jobs.remove(name);
				remove_model_files(&self.building_dir(), name)?;
				return Ok(());
			}
			None => (),
		}

		if Self::dat(&self.data_dir, name).exists() {
			let model = write(generator)?.remove_model(name);
			drop(model);
			remove_model_files(&self.data_dir, name)?;
		} else if Self::dat(&self.hidden_dir(), name).exists() {
			remove_model_files(&self.hidden_dir(), name)?;
		} else {
			return Err(AdminError::NotFound(format!("No model named '{}'", name)));
		}
		lock(&self.known).remove(name);
		Ok(())
	}

	/// Saves the text file of a new model and reserves its name, before the build.
	///
	/// # Errors
	/// Returns a conflict if the name is taken or if another model is being built:
	/// one build at a time, a build takes much memory.
	pub(crate) fn reserve(&self, name: &str, bytes: &[u8]) -> Result<(), AdminError> {
		let mut jobs = lock(&self.jobs);
		if jobs.values().any(|job| *job == Job::Building) {
			return Err(AdminError::Conflict("Another model is being built: wait for it to end".to_owned()));
		}
		if jobs.contains_key(name) || self.on_disk(name) {
			return Err(AdminError::Conflict(format!("A model named '{}' already exists", name)));
		}
		let building = self.building_dir();
		std::fs::create_dir_all(&building)?;
		// A cache left by an interrupted build would be read instead of the new text
		remove_model_files(&building, name)?;
		std::fs::write(Self::dat(&building, name), bytes)?;
		jobs.insert(name.to_owned(), Job::Building);
		Ok(())
	}

	/// Builds a reserved model, checks that it generates words, then shows it.
	/// Slow: run it on its own thread. On failure, the model stays listed as failed.
	pub(crate) fn build(&self, generator: &RwLock<Generator>, name: &str) {
		let result = self.build_and_show(generator, name);
		let mut jobs = lock(&self.jobs);
		match result {
			Ok(()) => {
				jobs.remove(name);
			}
			Err(error) => {
				let _ = remove_model_files(&self.building_dir(), name);
				jobs.insert(name.to_owned(), Job::Failed(error));
			}
		}
	}

	/// Gives up a reserved model whose build could not start: it is listed as failed.
	pub(crate) fn abandon(&self, name: &str, error: String) {
		let _ = remove_model_files(&self.building_dir(), name);
		lock(&self.jobs).insert(name.to_owned(), Job::Failed(error));
	}

	/// Steps of `build` that can fail.
	fn build_and_show(&self, generator: &RwLock<Generator>, name: &str) -> Result<(), String> {
		let building = self.building_dir();
		let model = MultiGramModel::new(Self::dat(&building, name)).map_err(|e| format!("Cannot build the model: {}", e))?;
		let model = smoke_test(model)?;
		let known = Known::of(&model);
		move_model(&building, &self.data_dir, name).map_err(|e| format!("Cannot move the model files: {:?}", e))?;
		lock(&self.known).insert(name.to_owned(), known);
		write(generator).map_err(|e| format!("{:?}", e))?.insert_model(model)
	}
}

/// Generates some words with the model alone, and gives it back if it works.
fn smoke_test(model: MultiGramModel) -> Result<MultiGramModel, String> {
	let name = model.get_name().to_owned();
	let mut probe = Generator::default();
	probe.insert_model(model)?;
	let mut input = probe.make_prediction_input();
	input.set_model_intensity(&name, 1.0)?;
	input.nb_try = 0;
	for _ in 0..SMOKE_TEST_WORDS {
		probe.predict(&input).map_err(|e| format!("The model does not generate words: {}", e))?;
	}
	probe.remove_model(&name).ok_or_else(|| "The model was lost during its test".to_owned())
}

/// Names of the `.dat` files of a directory, an absent directory having none.
fn dat_names(dir: &Path) -> Result<Vec<String>, AdminError> {
	let entries = match std::fs::read_dir(dir) {
		Ok(entries) => entries,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
		Err(e) => return Err(e.into()),
	};
	let mut names = Vec::new();
	for entry in entries {
		let path = entry?.path();
		if path.is_file()
			&& path.extension().is_some_and(|extension| extension == "dat")
			&& let Some(stem) = path.file_stem()
		{
			names.push(stem.to_string_lossy().into_owned());
		}
	}
	Ok(names)
}

/// Moves the text file and the binary cache of a model from one directory to another.
/// If the cache cannot follow, the text file goes back, so that the model is never split.
fn move_model(from: &Path, to: &Path, name: &str) -> Result<(), AdminError> {
	std::fs::rename(Catalog::dat(from, name), Catalog::dat(to, name))?;
	let bin = Catalog::bin(from, name);
	if bin.exists()
		&& let Err(e) = std::fs::rename(&bin, Catalog::bin(to, name))
	{
		std::fs::rename(Catalog::dat(to, name), Catalog::dat(from, name))?;
		return Err(e.into());
	}
	Ok(())
}

/// Removes the text file and the binary cache of a model, the absent ones being ignored.
fn remove_model_files(dir: &Path, name: &str) -> Result<(), AdminError> {
	for path in [Catalog::dat(dir, name), Catalog::bin(dir, name)] {
		match std::fs::remove_file(&path) {
			Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
			_ => (),
		}
	}
	Ok(())
}

/// Locks a table of the catalog. Each change is a single insert or remove, so a table
/// is never half written: a poisoned lock is used as is.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
	mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Locks the generator for a change.
fn write(generator: &RwLock<Generator>) -> Result<RwLockWriteGuard<'_, Generator>, AdminError> {
	generator.write().map_err(|_| AdminError::Internal("Model lock failed".to_owned()))
}

#[cfg(test)]
mod tests {
	use super::*;

	// A data directory dedicated to one test, with its shown models loaded
	struct Fixture {
		dir: PathBuf,
		generator: RwLock<Generator>,
		catalog: Catalog,
	}

	impl Drop for Fixture {
		fn drop(&mut self) {
			let _ = std::fs::remove_dir_all(&self.dir);
		}
	}

	fn fixture(test: &str, models: &[(&str, &str)]) -> Fixture {
		let dir = std::env::temp_dir().join(format!("rs-gen-catalog-{}-{}", std::process::id(), test));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		for (name, lines) in models {
			std::fs::write(dir.join(format!("{}.dat", name)), lines).unwrap();
		}
		let generator = Generator::new(&dir).unwrap();
		let catalog = Catalog::new(&dir, &generator);
		Fixture { dir, generator: RwLock::new(generator), catalog }
	}

	// Memory known in RAM only, the swap unknown
	fn ram(bytes: Option<u64>) -> Room {
		Room { ram_bytes: bytes, with_swap_bytes: None }
	}

	impl Fixture {
		fn status(&self, name: &str) -> Option<Status> {
			self.catalog.list().unwrap().into_iter().find(|model| model.name == name).map(|model| model.status)
		}

		fn loaded(&self, name: &str) -> bool {
			self.generator.read().unwrap().get_model(name).is_some()
		}
	}

	#[test]
	fn list_describes_the_shown_models() {
		let f = fixture("list", &[("fruits", "pomme\npoire\n"), ("legumes", "chou\n")]);
		let models = f.catalog.list().unwrap();
		assert_eq!(models.iter().map(|model| model.name.as_str()).collect::<Vec<_>>(), vec!["fruits", "legumes"]);
		let fruits = &models[0];
		assert_eq!(fruits.status, Status::Shown);
		assert_eq!(fruits.stats.unwrap().sentences, 2);
		assert_eq!(fruits.dat_bytes, Some(12));
		assert!(fruits.bin_bytes.is_some());
		assert!(fruits.estimated_bytes.unwrap() > 0);
	}

	#[test]
	fn hide_then_show_moves_the_files_and_the_model() {
		let f = fixture("hide-show", &[("fruits", "pomme\npoire\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		assert_eq!(f.status("fruits"), Some(Status::Hidden));
		assert!(!f.loaded("fruits"));
		assert!(f.dir.join("hidden/fruits.dat").exists() && f.dir.join("hidden/fruits.bin").exists());
		assert!(!f.dir.join("fruits.dat").exists());
		// Hidden models stay hidden after a restart: `Generator::new` ignores the sub-directories
		assert!(Generator::new(&f.dir).unwrap().get_model_names().is_empty());

		f.catalog.show(&f.generator, "fruits", Room::default(), false).unwrap();
		assert_eq!(f.status("fruits"), Some(Status::Shown));
		assert!(f.loaded("fruits"));
		assert!(f.dir.join("fruits.dat").exists() && !f.dir.join("hidden/fruits.dat").exists());
	}

	#[test]
	fn hide_and_show_refuse_the_wrong_state() {
		let f = fixture("wrong-state", &[("fruits", "pomme\n")]);
		assert!(matches!(f.catalog.show(&f.generator, "fruits", Room::default(), false), Err(AdminError::Conflict(_))));
		assert!(matches!(f.catalog.hide(&f.generator, "unknown"), Err(AdminError::NotFound(_))));
		f.catalog.hide(&f.generator, "fruits").unwrap();
		assert!(matches!(f.catalog.hide(&f.generator, "fruits"), Err(AdminError::Conflict(_))));
	}

	#[test]
	fn show_refuses_a_model_bigger_than_the_free_memory() {
		let f = fixture("show-memory", &[("fruits", "pomme\npoire\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		let result = f.catalog.show(&f.generator, "fruits", ram(Some(MEMORY_MARGIN_BYTES)), false);
		assert!(matches!(result, Err(AdminError::NotEnoughMemory(_))), "{:?}", result);
		assert_eq!(f.status("fruits"), Some(Status::Hidden));
		f.catalog.show(&f.generator, "fruits", ram(Some(2 * MEMORY_MARGIN_BYTES)), false).unwrap();
	}

	#[test]
	fn show_needs_force_for_a_model_that_fits_only_with_swap() {
		let f = fixture("show-swap", &[("fruits", "pomme\npoire\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		let room = Room { ram_bytes: Some(MEMORY_MARGIN_BYTES), with_swap_bytes: Some(2 * MEMORY_MARGIN_BYTES) };
		let result = f.catalog.show(&f.generator, "fruits", room, false);
		assert!(matches!(result, Err(AdminError::MemoryRisk(_))), "{:?}", result);
		assert_eq!(f.status("fruits"), Some(Status::Hidden));
		f.catalog.show(&f.generator, "fruits", room, true).unwrap();
		assert_eq!(f.status("fruits"), Some(Status::Shown));
		assert!(f.loaded("fruits"));
	}

	#[test]
	fn force_does_not_show_a_model_bigger_than_the_memory_with_swap() {
		let f = fixture("show-too-big", &[("fruits", "pomme\npoire\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		let room = Room { ram_bytes: Some(MEMORY_MARGIN_BYTES), with_swap_bytes: Some(MEMORY_MARGIN_BYTES) };
		let result = f.catalog.show(&f.generator, "fruits", room, true);
		assert!(matches!(result, Err(AdminError::NotEnoughMemory(_))), "{:?}", result);
		assert_eq!(f.status("fruits"), Some(Status::Hidden));
	}

	#[test]
	fn delete_removes_a_shown_or_hidden_model_for_good() {
		let f = fixture("delete", &[("fruits", "pomme\n"), ("legumes", "chou\n")]);
		f.catalog.delete(&f.generator, "fruits").unwrap();
		assert!(!f.loaded("fruits"));
		assert!(!f.dir.join("fruits.dat").exists() && !f.dir.join("fruits.bin").exists());

		f.catalog.hide(&f.generator, "legumes").unwrap();
		f.catalog.delete(&f.generator, "legumes").unwrap();
		assert!(!f.dir.join("hidden/legumes.dat").exists() && !f.dir.join("hidden/legumes.bin").exists());
		assert!(f.catalog.list().unwrap().is_empty());
		assert!(matches!(f.catalog.delete(&f.generator, "legumes"), Err(AdminError::NotFound(_))));
	}

	#[test]
	fn a_reserved_model_is_built_tested_and_shown() {
		let f = fixture("build", &[("fruits", "pomme\n")]);
		f.catalog.reserve("legumes", b"chou\ncarotte\n").unwrap();
		assert_eq!(f.status("legumes"), Some(Status::Building));
		assert!(f.catalog.name_taken("legumes"));
		f.catalog.build(&f.generator, "legumes");
		assert_eq!(f.status("legumes"), Some(Status::Shown));
		assert!(f.loaded("legumes"));
		assert!(f.dir.join("legumes.dat").exists() && f.dir.join("legumes.bin").exists());
		assert!(!f.dir.join("building/legumes.dat").exists());
	}

	#[test]
	fn reserve_refuses_a_taken_name_and_a_second_build() {
		let f = fixture("reserve", &[("fruits", "pomme\n")]);
		assert!(matches!(f.catalog.reserve("fruits", b"x\n"), Err(AdminError::Conflict(_))));
		f.catalog.reserve("legumes", b"chou\n").unwrap();
		assert!(matches!(f.catalog.reserve("fleurs", b"rose\n"), Err(AdminError::Conflict(_))));
		assert!(matches!(f.catalog.delete(&f.generator, "legumes"), Err(AdminError::Conflict(_))));
	}

	#[test]
	fn a_failed_build_is_listed_and_can_be_deleted() {
		let f = fixture("failed", &[]);
		f.catalog.reserve("bad", b"x\n").unwrap();
		// The text file vanishes before the build
		std::fs::remove_file(f.dir.join("building/bad.dat")).unwrap();
		f.catalog.build(&f.generator, "bad");
		let bad = f.catalog.list().unwrap().into_iter().find(|model| model.name == "bad").unwrap();
		assert_eq!(bad.status, Status::Failed);
		assert!(bad.error.is_some());
		assert!(!f.loaded("bad"));
		f.catalog.delete(&f.generator, "bad").unwrap();
		assert!(f.catalog.list().unwrap().is_empty());
		assert!(!f.catalog.name_taken("bad"));
	}
}
