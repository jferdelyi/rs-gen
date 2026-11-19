//! Model files of the data directory and their status: shown, hidden, or on their way.
//!
//! - A shown model is `data/<name>.dat` (and its cache `data/<name>.bin`), loaded in the generator.
//! - A hidden model is moved to `data/hidden/`: it is not loaded and takes no memory.
//! - A model being added is built in `data/building/`, then moved to `data/` once it works.
//! - A hidden model being shown is loaded from `data/hidden/`, then moved to `data/`.
//!
//! Next to each model, `<name>.stats` keeps its size once it was loaded, so that a hidden
//! model still shows its figures after a restart, and `<name>.label` its display name, if any:
//! the name shown to the visitors, while the technical name stays the same.
//!
//! `Generator::new` reads only the `.dat` files directly in `data/`: the sub-directories and
//! the other files are ignored.

use crate::check::{Fit, MEMORY_MARGIN_BYTES, fit, megabytes, megabytes_or_unknown};
use crate::system::Room;
use rs_gen_core::model::generator::Generator;
use rs_gen_core::model::multigram_model::{LearnProgress, ModelStats, MultiGramModel};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockWriteGuard};
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
	/// Learning its text file: being added, or shown without a binary cache.
	Learning,
	/// Writing its binary cache.
	Saving,
	/// Generating a few words to check that it works.
	Testing,
	/// Reading its binary cache, to be shown.
	Loading,
	/// Its addition failed; it can be deleted.
	Failed,
}

/// Description of a model, sent to the admin page.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub(crate) struct ModelInfo {
	/// Name of the model.
	pub(crate) name: String,

	/// Display name of the model, if any.
	pub(crate) label: Option<String>,

	/// Status of the model.
	pub(crate) status: Status,

	/// Share of the lines learned, from 0 to 1, while the model is learning.
	pub(crate) progress: Option<f64>,

	/// Why the addition failed, for a failed model; why the last show failed, for a hidden one.
	pub(crate) error: Option<String>,

	/// Size of the model, once it was loaded.
	pub(crate) stats: Option<ModelStats>,

	/// Number of transitions, once the model was loaded.
	pub(crate) transitions: Option<usize>,

	/// Estimated memory of the model once loaded, in bytes.
	pub(crate) estimated_bytes: Option<u64>,

	/// Size of the text file, in bytes.
	pub(crate) dat_bytes: Option<u64>,

	/// Size of the binary cache, in bytes.
	pub(crate) bin_bytes: Option<u64>,

	/// Last change of the text file or of the display name, in seconds since 1970.
	pub(crate) modified: Option<u64>,
}

/// Size of a model, known once it was loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

	/// Writes the size next to the model `name` of `dir`, as `<name>.stats`: one `key value` per line.
	fn save(&self, dir: &Path, name: &str) -> std::io::Result<()> {
		let stats = self.stats;
		let text = format!(
			"sentences {}\nsentence_bytes {}\nmax_n {}\nstates {}\nkey_bytes {}\ntransitions {}\n",
			stats.sentences, stats.sentence_bytes, stats.max_n, stats.states, stats.key_bytes, self.transitions
		);
		let path = Catalog::stats(dir, name);
		let temporary = path.with_extension("stats.tmp");
		std::fs::write(&temporary, text)?;
		std::fs::rename(&temporary, &path)
	}

	/// Reads the size written by `save`, `None` if there is none or if it is unreadable.
	fn read(dir: &Path, name: &str) -> Option<Self> {
		let text = std::fs::read_to_string(Catalog::stats(dir, name)).ok()?;
		let values: HashMap<&str, usize> = text
			.lines()
			.filter_map(|line| line.split_once(' '))
			.filter_map(|(key, value)| value.parse().ok().map(|value| (key, value)))
			.collect();
		let value = |key: &str| values.get(key).copied();
		Some(Self {
			stats: ModelStats {
				sentences: value("sentences")?,
				sentence_bytes: value("sentence_bytes")?,
				max_n: value("max_n")?,
				states: value("states")?,
				key_bytes: value("key_bytes")?,
			},
			transitions: value("transitions")?,
		})
	}
}

/// Step of a model being added or shown.
#[derive(Debug, Clone)]
enum Step {
	/// Learning its text file; the progress counts the lines learned.
	Learning(Arc<LearnProgress>),
	/// Writing its binary cache.
	Saving,
	/// Generating a few words to check that it works.
	Testing,
	/// Reading its binary cache.
	Loading,
	/// Its addition failed, for this reason.
	Failed(String),
}

/// A model being added, its files in `data/building/`, or a hidden model being shown,
/// its files in `data/hidden/`.
#[derive(Debug, Clone)]
struct Job {
	/// What it is doing.
	step: Step,
}

impl Job {
	/// Tells whether the job is still running: not failed.
	fn running(&self) -> bool {
		!matches!(self.step, Step::Failed(_))
	}

	/// Status, share of the lines learned and error to show for the model.
	fn describe(&self) -> (Status, Option<f64>, Option<String>) {
		match &self.step {
			Step::Learning(progress) => {
				let (done, total) = progress.lines();
				(Status::Learning, (total > 0).then(|| done as f64 / total as f64), None)
			}
			Step::Saving => (Status::Saving, None, None),
			Step::Testing => (Status::Testing, None, None),
			Step::Loading => (Status::Loading, None, None),
			Step::Failed(error) => (Status::Failed, None, Some(error.clone())),
		}
	}
}

/// Model files of the data directory and what is known about them.
pub(crate) struct Catalog {
	/// The data directory, read by `Generator::new`.
	data_dir: PathBuf,

	/// Size of the models loaded since the server started, shown or hidden since.
	known: Mutex<HashMap<String, Known>>,

	/// Models being added or shown, and the additions that failed.
	jobs: Mutex<HashMap<String, Job>>,

	/// Why the last show of a hidden model failed, by name.
	errors: Mutex<HashMap<String, String>>,
}

impl Catalog {
	/// Creates the catalog of `data_dir`, whose shown models are already loaded in `generator`,
	/// and writes their `.stats` files.
	pub(crate) fn new<P: AsRef<Path>>(data_dir: P, generator: &Generator) -> Self {
		let data_dir = data_dir.as_ref().to_path_buf();
		let known: HashMap<String, Known> = generator
			.get_model_names()
			.into_iter()
			.filter_map(|name| generator.get_model(&name).map(|model| (name, Known::of(model))))
			.collect();
		for (name, size) in &known {
			if Known::read(&data_dir, name) != Some(*size)
				&& let Err(e) = size.save(&data_dir, name)
			{
				eprintln!("admin: cannot write the size of '{}': {}", name, e);
			}
		}
		Self {
			data_dir,
			known: Mutex::new(known),
			jobs: Mutex::new(HashMap::new()),
			errors: Mutex::new(HashMap::new()),
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

	/// Size file of the model `name` in `dir`.
	fn stats(dir: &Path, name: &str) -> PathBuf {
		dir.join(format!("{}.stats", name))
	}

	/// Display name file of the model `name` in `dir`.
	fn label(dir: &Path, name: &str) -> PathBuf {
		dir.join(format!("{}.label", name))
	}

	/// Reads the display name of the model `name` of `dir`, `None` if it has none.
	fn read_label(dir: &Path, name: &str) -> Option<String> {
		let label = std::fs::read_to_string(Self::label(dir, name)).ok()?;
		let label = label.trim();
		(!label.is_empty()).then(|| label.to_owned())
	}

	/// Writes the display name of the model `name` of `dir`. An empty name leaves the file
	/// empty rather than removing it: its date records the removal, which the modification
	/// date of the model then shows.
	fn write_label(dir: &Path, name: &str, label: &str) -> std::io::Result<()> {
		let path = Self::label(dir, name);
		let temporary = path.with_extension("label.tmp");
		let text = if label.is_empty() { String::new() } else { format!("{}\n", label) };
		std::fs::write(&temporary, text)?;
		std::fs::rename(&temporary, &path)
	}

	/// Display names of the shown models, by name: what the generator page shows.
	pub(crate) fn labels(&self) -> HashMap<String, String> {
		dat_names(&self.data_dir)
			.unwrap_or_default()
			.into_iter()
			.filter_map(|name| Self::read_label(&self.data_dir, &name).map(|label| (name, label)))
			.collect()
	}

	/// Gives a display name to a shown or hidden model, or removes it for `None`: the file
	/// stays, empty, so that the removal dates the model.
	///
	/// # Errors
	/// Returns a conflict if the model is on its way or failed, not found if no model has this name.
	pub(crate) fn set_label(&self, name: &str, label: Option<&str>) -> Result<(), AdminError> {
		let jobs = lock(&self.jobs);
		if jobs.contains_key(name) {
			return Err(AdminError::Conflict(format!("The model '{}' is being added or shown, or its addition failed: wait for it to end", name)));
		}
		let dir = [self.data_dir.clone(), self.hidden_dir()]
			.into_iter()
			.find(|dir| Self::dat(dir, name).exists())
			.ok_or_else(|| AdminError::NotFound(format!("No model named '{}'", name)))?;
		Self::write_label(&dir, name, label.unwrap_or_default())?;
		Ok(())
	}

	/// Lists every model: shown, hidden, on its way or failed, sorted by name, each one once.
	///
	/// A model on its way moves between two directories: a list read during the move may
	/// find it in both or in neither. Its job then gives its status, and it is listed once.
	pub(crate) fn list(&self) -> Result<Vec<ModelInfo>, AdminError> {
		// Copied, not held: the other actions lock `jobs` before `known`
		let known = lock(&self.known).clone();
		let jobs = lock(&self.jobs).clone();
		let errors = lock(&self.errors).clone();
		let (data_dir, hidden_dir, building_dir) = (self.data_dir.clone(), self.hidden_dir(), self.building_dir());
		let mut names: BTreeSet<String> = jobs.keys().cloned().collect();
		names.extend(dat_names(&data_dir)?);
		names.extend(dat_names(&hidden_dir)?);

		let mut models = Vec::new();
		for name in names {
			// Where its files are now, if anywhere
			let dir = [&data_dir, &hidden_dir, &building_dir].into_iter().find(|dir| Self::dat(dir, &name).exists());
			let (status, progress, error) = match (jobs.get(&name), dir) {
				(Some(job), _) => job.describe(),
				(None, Some(dir)) if *dir == data_dir => (Status::Shown, None, None),
				(None, Some(dir)) if *dir == hidden_dir => (Status::Hidden, None, errors.get(&name).cloned()),
				// Moved or deleted since the directory was read, or a file left in `building/`
				(None, _) => continue,
			};
			let dir = dir.unwrap_or(&data_dir);
			let size = known.get(&name).copied().or_else(|| Known::read(dir, &name));
			let dat = std::fs::metadata(Self::dat(dir, &name)).ok();
			models.push(ModelInfo {
				label: Self::read_label(dir, &name),
				status,
				progress,
				error,
				stats: size.map(|size| size.stats),
				transitions: size.map(|size| size.transitions),
				estimated_bytes: size.map(|size| size.stats.estimated_bytes()),
				dat_bytes: dat.as_ref().map(|metadata| metadata.len()),
				bin_bytes: std::fs::metadata(Self::bin(dir, &name)).ok().map(|metadata| metadata.len()),
				modified: Self::modified(dir, &name),
				name,
			});
		}
		Ok(models)
	}

	/// Last change of the model `name` of `dir`, in seconds since 1970: of its text file,
	/// or of its display name if it changed since. Moving the files keeps these dates.
	fn modified(dir: &Path, name: &str) -> Option<u64> {
		[Self::dat(dir, name), Self::label(dir, name)]
			.iter()
			.filter_map(|path| std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok())
			.max()
			.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
			.map(|duration| duration.as_secs())
	}

	/// Tells whether a model, shown, hidden or being added, already has this name.
	pub(crate) fn name_taken(&self, name: &str) -> bool {
		lock(&self.jobs).contains_key(name) || self.on_disk(name)
	}

	/// Tells whether the model is shown or hidden.
	fn on_disk(&self, name: &str) -> bool {
		Self::dat(&self.data_dir, name).exists() || Self::dat(&self.hidden_dir(), name).exists()
	}

	/// Changes the step of a running job.
	fn set_step(&self, name: &str, step: Step) {
		if let Some(job) = lock(&self.jobs).get_mut(name) {
			job.step = step;
		}
	}

	/// Error for an action that needs the memory while another model is being added or shown.
	fn busy() -> AdminError {
		AdminError::Conflict("Another model is being added or shown: wait for it to end".to_owned())
	}

	/// Hides a shown model: its files go to `data/hidden/` and it leaves the memory.
	pub(crate) fn hide(&self, generator: &RwLock<Generator>, name: &str) -> Result<(), AdminError> {
		let jobs = lock(&self.jobs);
		if jobs.get(name).is_some_and(Job::running) {
			return Err(AdminError::Conflict(format!("The model '{}' is being shown: wait for it to end", name)));
		}
		if !Self::dat(&self.data_dir, name).exists() {
			return Err(self.not_shown(name));
		}
		std::fs::create_dir_all(self.hidden_dir())?;
		move_model(&self.data_dir, &self.hidden_dir(), name)?;
		lock(&self.errors).remove(name);
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

	/// Starts showing a hidden model, if the server has the memory for it: the model is
	/// listed as loading until `load`, run on its own thread, ends.
	///
	/// # Parameters
	/// - `room`: memory the server can still give, in RAM and with swap.
	/// - `force`: shows the model even if it fits only by using swap.
	pub(crate) fn show(&self, name: &str, room: Room, force: bool) -> Result<(), AdminError> {
		let mut jobs = lock(&self.jobs);
		if jobs.get(name).is_some_and(Job::running) {
			return Err(AdminError::Conflict(format!("The model '{}' is already being shown", name)));
		}
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
		// One model added or shown at a time: each one takes much memory
		if jobs.values().any(Job::running) {
			return Err(Self::busy());
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

		jobs.insert(name.to_owned(), Job { step: Step::Loading });
		lock(&self.errors).remove(name);
		Ok(())
	}

	/// Loads a model that `show` started to show, then moves it to `data/`.
	/// Slow: run it on its own thread. On failure, the model stays hidden, with the reason.
	pub(crate) fn load(&self, generator: &RwLock<Generator>, name: &str) {
		let result = self.load_and_show(generator, name);
		let mut jobs = lock(&self.jobs);
		jobs.remove(name);
		match result {
			Ok(()) => eprintln!("admin: model '{}' loaded", name),
			Err(error) => {
				eprintln!("admin: model '{}' not loaded: {}", name, error);
				lock(&self.errors).insert(name.to_owned(), error);
			}
		}
	}

	/// Gives up a show that `show` started but whose loading could not start.
	pub(crate) fn abandon_show(&self, name: &str, error: String) {
		let mut jobs = lock(&self.jobs);
		jobs.remove(name);
		lock(&self.errors).insert(name.to_owned(), error);
	}

	/// Steps of `load` that can fail.
	fn load_and_show(&self, generator: &RwLock<Generator>, name: &str) -> Result<(), String> {
		let hidden = self.hidden_dir();
		let dat = Self::dat(&hidden, name);
		let model = match MultiGramModel::from_cache(&dat).map_err(|e| format!("Cannot read the model file: {}", e))? {
			Some(model) => model,
			// No cache: the model is learned from its text file, then its cache is written
			None => {
				let progress = Arc::new(LearnProgress::default());
				self.set_step(name, Step::Learning(Arc::clone(&progress)));
				let model = MultiGramModel::learn_file(&dat, &progress).map_err(|e| format!("Cannot learn the model: {}", e))?;
				self.set_step(name, Step::Saving);
				model.save_cache(&dat).map_err(|e| format!("Cannot write the model file: {}", e))?;
				model
			}
		};
		let known = Known::of(&model);
		if let Err(e) = known.save(&hidden, name) {
			eprintln!("admin: cannot write the size of '{}': {}", name, e);
		}
		move_model(&hidden, &self.data_dir, name).map_err(|e| format!("Cannot move the model files: {:?}", e))?;
		lock(&self.known).insert(name.to_owned(), known);
		let inserted = write(generator).map_err(|e| format!("{:?}", e)).and_then(|mut generator| generator.insert_model(model));
		if let Err(error) = inserted {
			let _ = move_model(&self.data_dir, &hidden, name);
			return Err(error);
		}
		Ok(())
	}

	/// Estimated peak memory to load the model `name` of `dir`.
	///
	/// Its size is known if it was loaded once, counted from its text file otherwise.
	/// Without a binary cache, the model is built: the peak is the one of a build.
	fn needed_to_load(&self, dir: &Path, name: &str) -> Result<u64, AdminError> {
		let dat_path = Self::dat(dir, name);
		let known = lock(&self.known).get(name).copied().or_else(|| Known::read(dir, name));
		let stats = match known {
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
			Some(job) if job.running() => {
				return Err(AdminError::Conflict(format!("The model '{}' is being added or shown: wait for it to end", name)));
			}
			Some(_) => {
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
		lock(&self.errors).remove(name);
		Ok(())
	}

	/// Saves the text file of a new model and reserves its name, before the build.
	///
	/// # Errors
	/// Returns a conflict if the name is taken or if another model is being added or shown:
	/// one at a time, each one takes much memory.
	pub(crate) fn reserve(&self, name: &str, bytes: &[u8], label: Option<&str>) -> Result<(), AdminError> {
		let mut jobs = lock(&self.jobs);
		if jobs.values().any(Job::running) {
			return Err(Self::busy());
		}
		if jobs.contains_key(name) || self.on_disk(name) {
			return Err(AdminError::Conflict(format!("A model named '{}' already exists", name)));
		}
		let building = self.building_dir();
		std::fs::create_dir_all(&building)?;
		// A cache left by an interrupted build would be read instead of the new text
		remove_model_files(&building, name)?;
		std::fs::write(Self::dat(&building, name), bytes)?;
		// A model added without a display name has no display name file
		if let Some(label) = label {
			Self::write_label(&building, name, label)?;
		}
		jobs.insert(name.to_owned(), Job { step: Step::Learning(Arc::new(LearnProgress::default())) });
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
				eprintln!("admin: model '{}' built", name);
			}
			Err(error) => {
				eprintln!("admin: model '{}' not built: {}", name, error);
				let _ = remove_model_files(&self.building_dir(), name);
				jobs.insert(name.to_owned(), Job { step: Step::Failed(error) });
			}
		}
	}

	/// Gives up a reserved model whose build could not start: it is listed as failed.
	pub(crate) fn abandon(&self, name: &str, error: String) {
		let _ = remove_model_files(&self.building_dir(), name);
		lock(&self.jobs).insert(name.to_owned(), Job { step: Step::Failed(error) });
	}

	/// Steps of `build` that can fail.
	fn build_and_show(&self, generator: &RwLock<Generator>, name: &str) -> Result<(), String> {
		let building = self.building_dir();
		let dat = Self::dat(&building, name);
		let progress = Arc::new(LearnProgress::default());
		self.set_step(name, Step::Learning(Arc::clone(&progress)));
		let model = MultiGramModel::learn_file(&dat, &progress).map_err(|e| format!("Cannot build the model: {}", e))?;
		self.set_step(name, Step::Saving);
		model.save_cache(&dat).map_err(|e| format!("Cannot write the model file: {}", e))?;
		self.set_step(name, Step::Testing);
		let model = smoke_test(model)?;
		let known = Known::of(&model);
		if let Err(e) = known.save(&building, name) {
			eprintln!("admin: cannot write the size of '{}': {}", name, e);
		}
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
/// The size file follows if it can: without it, the size is only unknown until the next load.
/// The display name follows too; if it cannot, it stays where it was and the move is logged.
fn move_model(from: &Path, to: &Path, name: &str) -> Result<(), AdminError> {
	std::fs::rename(Catalog::dat(from, name), Catalog::dat(to, name))?;
	let bin = Catalog::bin(from, name);
	if bin.exists()
		&& let Err(e) = std::fs::rename(&bin, Catalog::bin(to, name))
	{
		std::fs::rename(Catalog::dat(to, name), Catalog::dat(from, name))?;
		return Err(e.into());
	}
	let stats = Catalog::stats(from, name);
	if stats.exists() && std::fs::rename(&stats, Catalog::stats(to, name)).is_err() {
		let _ = std::fs::remove_file(&stats);
	}
	let label = Catalog::label(from, name);
	if label.exists()
		&& let Err(e) = std::fs::rename(&label, Catalog::label(to, name))
	{
		eprintln!("admin: the display name of '{}' did not follow the model: {}", name, e);
	}
	Ok(())
}

/// Removes the text file, the binary cache, the size file and the display name of a model,
/// the absent ones being ignored.
fn remove_model_files(dir: &Path, name: &str) -> Result<(), AdminError> {
	for path in [Catalog::dat(dir, name), Catalog::bin(dir, name), Catalog::stats(dir, name), Catalog::label(dir, name)] {
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

		// Shows a hidden model as the route does, the loading run here instead of on its own thread
		fn show(&self, name: &str, room: Room, force: bool) -> Result<(), AdminError> {
			self.catalog.show(name, room, force)?;
			self.catalog.load(&self.generator, name);
			Ok(())
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

		f.show("fruits", Room::default(), false).unwrap();
		assert_eq!(f.status("fruits"), Some(Status::Shown));
		assert!(f.loaded("fruits"));
		assert!(f.dir.join("fruits.dat").exists() && !f.dir.join("hidden/fruits.dat").exists());
	}

	#[test]
	fn hide_and_show_refuse_the_wrong_state() {
		let f = fixture("wrong-state", &[("fruits", "pomme\n")]);
		assert!(matches!(f.show("fruits", Room::default(), false), Err(AdminError::Conflict(_))));
		assert!(matches!(f.catalog.hide(&f.generator, "unknown"), Err(AdminError::NotFound(_))));
		f.catalog.hide(&f.generator, "fruits").unwrap();
		assert!(matches!(f.catalog.hide(&f.generator, "fruits"), Err(AdminError::Conflict(_))));
	}

	#[test]
	fn show_refuses_a_model_bigger_than_the_free_memory() {
		let f = fixture("show-memory", &[("fruits", "pomme\npoire\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		let result = f.show("fruits", ram(Some(MEMORY_MARGIN_BYTES)), false);
		assert!(matches!(result, Err(AdminError::NotEnoughMemory(_))), "{:?}", result);
		assert_eq!(f.status("fruits"), Some(Status::Hidden));
		f.show("fruits", ram(Some(2 * MEMORY_MARGIN_BYTES)), false).unwrap();
	}

	#[test]
	fn show_needs_force_for_a_model_that_fits_only_with_swap() {
		let f = fixture("show-swap", &[("fruits", "pomme\npoire\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		let room = Room { ram_bytes: Some(MEMORY_MARGIN_BYTES), with_swap_bytes: Some(2 * MEMORY_MARGIN_BYTES) };
		let result = f.show("fruits", room, false);
		assert!(matches!(result, Err(AdminError::MemoryRisk(_))), "{:?}", result);
		assert_eq!(f.status("fruits"), Some(Status::Hidden));
		f.show("fruits", room, true).unwrap();
		assert_eq!(f.status("fruits"), Some(Status::Shown));
		assert!(f.loaded("fruits"));
	}

	#[test]
	fn force_does_not_show_a_model_bigger_than_the_memory_with_swap() {
		let f = fixture("show-too-big", &[("fruits", "pomme\npoire\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		let room = Room { ram_bytes: Some(MEMORY_MARGIN_BYTES), with_swap_bytes: Some(MEMORY_MARGIN_BYTES) };
		let result = f.show("fruits", room, true);
		assert!(matches!(result, Err(AdminError::NotEnoughMemory(_))), "{:?}", result);
		assert_eq!(f.status("fruits"), Some(Status::Hidden));
	}

	// The display name of `name` in the list
	fn label_of(f: &Fixture, name: &str) -> Option<String> {
		f.catalog.list().unwrap().into_iter().find(|model| model.name == name).and_then(|model| model.label)
	}

	#[test]
	fn a_display_name_is_saved_listed_and_removed() {
		let f = fixture("label", &[("fruits", "pomme\n")]);
		f.catalog.set_label("fruits", Some("Fruits d'été")).unwrap();
		assert_eq!(label_of(&f, "fruits").as_deref(), Some("Fruits d'été"));
		assert_eq!(f.catalog.labels().get("fruits").map(String::as_str), Some("Fruits d'été"));
		// The technical name stays the same
		assert!(f.loaded("fruits") && f.dir.join("fruits.dat").exists());
		f.catalog.set_label("fruits", None).unwrap();
		assert_eq!(label_of(&f, "fruits"), None);
		// The file stays, empty: its date records the removal
		assert_eq!(std::fs::read_to_string(f.dir.join("fruits.label")).unwrap(), "");
		assert!(f.catalog.labels().is_empty());
	}

	#[test]
	fn a_new_display_name_changes_the_modification_date() {
		let f = fixture("label-date", &[("fruits", "pomme\n")]);
		// A text file written long ago
		let long_ago = UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
		std::fs::File::options().write(true).open(f.dir.join("fruits.dat")).unwrap().set_modified(long_ago).unwrap();
		let modified = |f: &Fixture| f.catalog.list().unwrap().into_iter().find(|model| model.name == "fruits").unwrap().modified;
		assert_eq!(modified(&f), Some(1_600_000_000));
		f.catalog.set_label("fruits", Some("Fruits")).unwrap();
		assert!(modified(&f).unwrap() > 1_600_000_000, "{:?}", modified(&f));
	}

	#[test]
	fn removing_the_display_name_changes_the_modification_date() {
		let f = fixture("label-removed-date", &[("fruits", "pomme\n")]);
		// A text file and a display name both written long ago
		let long_ago = UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
		f.catalog.set_label("fruits", Some("Fruits")).unwrap();
		for file in ["fruits.dat", "fruits.label"] {
			std::fs::File::options().write(true).open(f.dir.join(file)).unwrap().set_modified(long_ago).unwrap();
		}
		let modified = |f: &Fixture| f.catalog.list().unwrap().into_iter().find(|model| model.name == "fruits").unwrap().modified;
		assert_eq!(modified(&f), Some(1_600_000_000));
		f.catalog.set_label("fruits", None).unwrap();
		assert_eq!(label_of(&f, "fruits"), None);
		assert!(modified(&f).unwrap() > 1_600_000_000, "{:?}", modified(&f));
	}

	#[test]
	fn the_display_name_follows_the_model_and_goes_with_it() {
		let f = fixture("label-moves", &[("fruits", "pomme\n")]);
		f.catalog.set_label("fruits", Some("Fruits")).unwrap();
		f.catalog.hide(&f.generator, "fruits").unwrap();
		assert!(f.dir.join("hidden/fruits.label").exists());
		assert_eq!(label_of(&f, "fruits").as_deref(), Some("Fruits"));
		// Only the shown models reach the generator page
		assert!(f.catalog.labels().is_empty());
		f.show("fruits", Room::default(), false).unwrap();
		assert_eq!(f.catalog.labels().get("fruits").map(String::as_str), Some("Fruits"));
		f.catalog.delete(&f.generator, "fruits").unwrap();
		assert!(!f.dir.join("fruits.label").exists());
	}

	#[test]
	fn a_new_model_keeps_the_display_name_given_at_its_addition() {
		let f = fixture("label-added", &[]);
		f.catalog.reserve("legumes", b"chou\n", Some("Légumes")).unwrap();
		assert_eq!(label_of(&f, "legumes").as_deref(), Some("Légumes"));
		f.catalog.build(&f.generator, "legumes");
		assert_eq!(f.status("legumes"), Some(Status::Shown));
		assert_eq!(label_of(&f, "legumes").as_deref(), Some("Légumes"));
		assert!(f.dir.join("legumes.label").exists());
	}

	#[test]
	fn set_label_refuses_a_model_on_its_way_or_unknown() {
		let f = fixture("label-refused", &[]);
		f.catalog.reserve("legumes", b"chou\n", None).unwrap();
		assert!(matches!(f.catalog.set_label("legumes", Some("Légumes")), Err(AdminError::Conflict(_))));
		assert!(matches!(f.catalog.set_label("absent", Some("Absent")), Err(AdminError::NotFound(_))));
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
		f.catalog.reserve("legumes", b"chou\ncarotte\n", None).unwrap();
		assert_eq!(f.status("legumes"), Some(Status::Learning));
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
		assert!(matches!(f.catalog.reserve("fruits", b"x\n", None), Err(AdminError::Conflict(_))));
		f.catalog.reserve("legumes", b"chou\n", None).unwrap();
		assert!(matches!(f.catalog.reserve("fleurs", b"rose\n", None), Err(AdminError::Conflict(_))));
		assert!(matches!(f.catalog.delete(&f.generator, "legumes"), Err(AdminError::Conflict(_))));
	}

	// Size of a model as the list shows it
	fn size_of(f: &Fixture, name: &str) -> (Option<ModelStats>, Option<usize>) {
		let model = f.catalog.list().unwrap().into_iter().find(|model| model.name == name).unwrap();
		(model.stats, model.transitions)
	}

	#[test]
	fn a_hidden_model_keeps_its_size_after_a_restart() {
		let f = fixture("size-restart", &[("fruits", "pomme\npoire\n")]);
		let shown = size_of(&f, "fruits");
		assert!(shown.0.is_some() && f.dir.join("fruits.stats").exists());
		f.catalog.hide(&f.generator, "fruits").unwrap();
		assert!(f.dir.join("hidden/fruits.stats").exists() && !f.dir.join("fruits.stats").exists());

		// A restart: the hidden model was never loaded by this catalog
		let restarted = Fixture {
			dir: f.dir.clone(),
			generator: RwLock::new(Generator::new(&f.dir).unwrap()),
			catalog: Catalog::new(&f.dir, &Generator::new(&f.dir).unwrap()),
		};
		assert_eq!(size_of(&restarted, "fruits"), shown);
		restarted.catalog.delete(&restarted.generator, "fruits").unwrap();
		assert!(!f.dir.join("hidden/fruits.stats").exists());
	}

	#[test]
	fn the_size_file_is_read_back_and_an_unreadable_one_is_ignored() {
		let f = fixture("size-file", &[]);
		let size = Known { stats: ModelStats { sentences: 3, sentence_bytes: 15, max_n: 7, states: 42, key_bytes: 99 }, transitions: 50 };
		size.save(&f.dir, "words").unwrap();
		assert_eq!(Known::read(&f.dir, "words"), Some(size));
		std::fs::write(f.dir.join("words.stats"), "sentences 3\nstates x\n").unwrap();
		assert_eq!(Known::read(&f.dir, "words"), None);
		assert_eq!(Known::read(&f.dir, "absent"), None);
	}

	#[test]
	fn show_lists_the_model_as_loading_until_it_is_loaded() {
		let f = fixture("show-loading", &[("fruits", "pomme\npoire\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		f.catalog.show("fruits", Room::default(), false).unwrap();
		assert_eq!(f.status("fruits"), Some(Status::Loading));
		assert!(!f.loaded("fruits"));
		// Not twice, and nothing else may take the memory meanwhile
		assert!(matches!(f.catalog.show("fruits", Room::default(), false), Err(AdminError::Conflict(_))));
		assert!(matches!(f.catalog.reserve("legumes", b"chou\n", None), Err(AdminError::Conflict(_))));
		assert!(matches!(f.catalog.delete(&f.generator, "fruits"), Err(AdminError::Conflict(_))));
		f.catalog.load(&f.generator, "fruits");
		assert_eq!(f.status("fruits"), Some(Status::Shown));
		assert!(f.loaded("fruits"));
	}

	#[test]
	fn show_without_cache_learns_the_model_and_writes_its_cache() {
		let f = fixture("show-learn", &[("fruits", "pomme\npoire\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		std::fs::remove_file(f.dir.join("hidden/fruits.bin")).unwrap();
		f.show("fruits", Room::default(), false).unwrap();
		assert_eq!(f.status("fruits"), Some(Status::Shown));
		assert!(f.dir.join("fruits.bin").exists());
	}

	#[test]
	fn a_failed_show_keeps_the_model_hidden_with_the_reason() {
		let f = fixture("show-failed", &[("fruits", "pomme\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		std::fs::remove_file(f.dir.join("hidden/fruits.bin")).unwrap();
		// Not UTF-8: the text cannot be learned
		std::fs::write(f.dir.join("hidden/fruits.dat"), [0xff, 0xfe]).unwrap();
		f.show("fruits", Room::default(), false).unwrap();
		let fruits = f.catalog.list().unwrap().into_iter().find(|model| model.name == "fruits").unwrap();
		assert_eq!(fruits.status, Status::Hidden);
		assert!(fruits.error.is_some());
		assert!(!f.loaded("fruits"));
		assert!(f.dir.join("hidden/fruits.dat").exists());
	}

	#[test]
	fn a_model_file_that_cannot_be_written_is_named_as_on_the_admin_page() {
		// The admin page calls the .bin file the model file: the reason of a failure does too.
		// A directory where the model file is first written makes the write fail.
		let f = fixture("save-failed", &[("fruits", "pomme\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		std::fs::remove_file(f.dir.join("hidden/fruits.bin")).unwrap();
		std::fs::create_dir(f.dir.join("hidden/fruits.bin.tmp")).unwrap();
		f.show("fruits", Room::default(), false).unwrap();
		f.catalog.reserve("legumes", b"chou\n", None).unwrap();
		std::fs::create_dir(f.dir.join("building/legumes.bin.tmp")).unwrap();
		f.catalog.build(&f.generator, "legumes");
		for name in ["fruits", "legumes"] {
			let error = entries(&f, name).remove(0).error.unwrap_or_else(|| panic!("{} has no reason", name));
			assert!(error.starts_with("Cannot write the model file: "), "{}: {}", name, error);
		}
	}

	// The entries of the list named `name`
	fn entries(f: &Fixture, name: &str) -> Vec<ModelInfo> {
		f.catalog.list().unwrap().into_iter().filter(|model| model.name == name).collect()
	}

	#[test]
	fn a_model_moving_out_of_hidden_is_still_listed() {
		// A list read during the move may find the text file in neither directory
		let f = fixture("list-moving", &[("fruits", "pomme\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		f.catalog.show("fruits", Room::default(), false).unwrap();
		std::fs::rename(f.dir.join("hidden/fruits.dat"), f.dir.join("fruits.moving")).unwrap();
		let listed = entries(&f, "fruits");
		assert_eq!(listed.len(), 1, "{:?}", listed);
		assert_eq!(listed[0].status, Status::Loading);
	}

	#[test]
	fn a_model_moving_into_data_is_listed_once() {
		// At the end of an addition, the files are in `data/` before the job ends
		let f = fixture("list-twice", &[]);
		f.catalog.reserve("legumes", b"chou\n", None).unwrap();
		std::fs::copy(f.dir.join("building/legumes.dat"), f.dir.join("legumes.dat")).unwrap();
		let listed = entries(&f, "legumes");
		assert_eq!(listed.len(), 1, "{:?}", listed);
		assert_eq!(listed[0].status, Status::Learning);
	}

	#[test]
	fn a_build_blocks_the_show_of_another_model() {
		let f = fixture("one-at-a-time", &[("fruits", "pomme\n")]);
		f.catalog.hide(&f.generator, "fruits").unwrap();
		f.catalog.reserve("legumes", b"chou\n", None).unwrap();
		assert!(matches!(f.catalog.show("fruits", Room::default(), false), Err(AdminError::Conflict(_))));
		f.catalog.build(&f.generator, "legumes");
		f.show("fruits", Room::default(), false).unwrap();
		assert_eq!(f.status("fruits"), Some(Status::Shown));
	}

	#[test]
	fn learning_tells_the_share_of_the_lines_learned() {
		let f = fixture("progress", &[]);
		let dat = f.dir.join("words.dat");
		std::fs::write(&dat, "a\nb\nc\n").unwrap();
		let progress = Arc::new(LearnProgress::default());
		let job = Job { step: Step::Learning(Arc::clone(&progress)) };
		// The file is not read yet: no share
		assert_eq!(job.describe(), (Status::Learning, None, None));
		MultiGramModel::learn_file(&dat, &progress).unwrap();
		assert_eq!(job.describe(), (Status::Learning, Some(1.0), None));
	}

	#[test]
	fn a_failed_build_is_listed_and_can_be_deleted() {
		let f = fixture("failed", &[]);
		f.catalog.reserve("bad", b"x\n", None).unwrap();
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
