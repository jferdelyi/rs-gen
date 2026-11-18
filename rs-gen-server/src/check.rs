//! Checks a text file sent by the admin before it becomes a model: name, content, and
//! whether the server has the memory to build it and keep it.
//!
//! A model that fits in RAM passes. A model that fits only by using swap is a risk: the
//! admin may force it, the generation is then slower. A model that does not fit even
//! with swap is refused, forced or not: the service would be killed for lack of memory,
//! restarted, and killed again while loading the same model.

use crate::system::Room;
use rs_gen_core::model::multigram_model::{MultiGramModel, TextCount};
use serde::Serialize;

/// Largest text file accepted, in bytes (32 MB, the unit of the admin page).
pub(crate) const MAX_UPLOAD_BYTES: usize = 32_000_000;

/// Longest line accepted, in characters: a line of L characters gives about L² / 2 states.
pub(crate) const MAX_LINE_CHARS: usize = 256;

/// Memory kept free for the rest of the server (requests, nginx, the system): 64 MB.
pub(crate) const MEMORY_MARGIN_BYTES: u64 = 64_000_000;

/// States counted at most when the free memory is unknown (about 2 GB of model).
const MAX_COUNTED_STATES: usize = 20_000_000;

/// Every state costs more than this to be learned (the hash table of the learning, at least
/// 1.5 times 88 bytes in the estimate): counting stops beyond the free memory divided by it,
/// since such a model cannot be added.
const MIN_BYTES_PER_STATE: u64 = 64;

/// Longest model name.
const MAX_NAME_CHARS: usize = 64;

/// Longest display name of a model, in characters.
pub(crate) const MAX_LABEL_CHARS: usize = 64;

/// Result of a check, sent to the admin page.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckReport {
	/// Name of the model.
	pub(crate) name: String,

	/// True when nothing prevents adding the model, nor needs to be forced.
	pub(crate) ok: bool,

	/// What prevents adding the model, forced or not.
	pub(crate) problems: Vec<String>,

	/// What prevents adding the model unless the admin forces it: it fits only by using swap.
	pub(crate) risks: Vec<String>,

	/// What does not prevent adding the model but deserves a look.
	pub(crate) warnings: Vec<String>,

	/// What the text would give, if it could be read.
	pub(crate) count: Option<TextCount>,

	/// Estimated memory of the model once loaded, in bytes.
	pub(crate) estimated_bytes: Option<u64>,

	/// Estimated peak memory while building it, in bytes.
	pub(crate) needed_bytes: Option<u64>,

	/// Memory the server can still give in RAM, in bytes, if known.
	pub(crate) available_bytes: Option<u64>,

	/// Memory the server can still give in RAM and swap, in bytes, if known.
	pub(crate) available_with_swap_bytes: Option<u64>,
}

/// How a model fits in the memory left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fit {
	/// It fits in RAM, margin included.
	InRam,
	/// It fits only by using swap: allowed when the admin forces it.
	WithSwap,
	/// It does not fit even with swap: always refused.
	TooBig,
	/// The memory left is unknown: not checked.
	Unknown,
}

/// Tells how a model needing `needed` bytes fits in `room`, keeping `MEMORY_MARGIN_BYTES` free.
pub(crate) fn fit(needed: u64, room: Room) -> Fit {
	let with_margin = needed.saturating_add(MEMORY_MARGIN_BYTES);
	match (room.ram_bytes, room.with_swap_bytes) {
		(None, None) => Fit::Unknown,
		(Some(ram), _) if with_margin <= ram => Fit::InRam,
		(_, Some(with_swap)) if with_margin <= with_swap => Fit::WithSwap,
		_ => Fit::TooBig,
	}
}

/// Formats a memory that may be unknown.
pub(crate) fn megabytes_or_unknown(bytes: Option<u64>) -> String {
	bytes.map_or_else(|| "an unknown amount".to_owned(), megabytes)
}

/// Tells whether `name` can name a model: 1 to 64 lowercase letters, digits, `_` or `-`,
/// starting with a letter or a digit. Such a name is also a safe file name.
pub(crate) fn is_valid_name(name: &str) -> bool {
	let mut chars = name.chars();
	chars.next().is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
		&& name.len() <= MAX_NAME_CHARS
		&& name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Checks the display name of a model: any characters but control ones (a line break, a tab),
/// at most `MAX_LABEL_CHARS` once trimmed.
///
/// # Returns
/// - `Ok(None)` for an empty name: the model shows its technical name.
/// - `Ok(Some(label))` for the trimmed name.
///
/// # Errors
/// Returns the message to show for a name too long or with a control character.
pub(crate) fn check_label(label: &str) -> Result<Option<String>, String> {
	let label = label.trim();
	if label.is_empty() {
		return Ok(None);
	}
	if label.chars().count() > MAX_LABEL_CHARS {
		return Err(format!("The display name has more than {} characters", MAX_LABEL_CHARS));
	}
	if label.chars().any(char::is_control) {
		return Err("The display name holds a line break, a tab or another control character".to_owned());
	}
	Ok(Some(label.to_owned()))
}

/// Formats a number of bytes for a message, in megabytes, as the admin page does.
pub(crate) fn megabytes(bytes: u64) -> String {
	if bytes > 0 && bytes < 10_000 {
		return "< 0.01 MB".to_owned();
	}
	format!("{:.2} MB", bytes as f64 / 1_000_000.0)
}

/// Checks a text file before it becomes the model `name`.
///
/// # Parameters
/// - `name_taken`: true if a model, shown or hidden, already has this name.
/// - `bytes`: the content of the file.
/// - `room`: memory the server can still give, in RAM and with swap.
pub(crate) fn check_upload(name: &str, name_taken: bool, bytes: &[u8], room: Room) -> CheckReport {
	let mut report = CheckReport {
		name: name.to_owned(),
		ok: false,
		problems: Vec::new(),
		risks: Vec::new(),
		warnings: Vec::new(),
		count: None,
		estimated_bytes: None,
		needed_bytes: None,
		available_bytes: room.ram_bytes,
		available_with_swap_bytes: room.with_swap_bytes,
	};

	if !is_valid_name(name) {
		report.problems.push(format!(
			"Invalid name: use 1 to {} lowercase letters, digits, '_' or '-', starting with a letter or a digit",
			MAX_NAME_CHARS
		));
	} else if name_taken {
		report.problems.push(format!("A model named '{}' already exists", name));
	}

	if bytes.len() > MAX_UPLOAD_BYTES {
		report.problems.push(format!("The file is too big: {}, at most {}", megabytes(bytes.len() as u64), megabytes(MAX_UPLOAD_BYTES as u64)));
		return report;
	}
	let Ok(text) = std::str::from_utf8(bytes) else {
		report.problems.push("The file is not UTF-8 text".to_owned());
		return report;
	};

	// Counting stops as soon as the model cannot fit, even with swap, so a huge file does not
	// exhaust the memory. Counting a state takes less memory than keeping it.
	let largest_room = room.with_swap_bytes.or(room.ram_bytes);
	let max_states = match largest_room {
		Some(available) => (available.saturating_sub(MEMORY_MARGIN_BYTES) / MIN_BYTES_PER_STATE) as usize,
		None => MAX_COUNTED_STATES,
	};
	let count = MultiGramModel::count_text(text, max_states);
	report.count = Some(count);

	if count.lines == count.empty_lines {
		report.problems.push("The file has no word".to_owned());
	}
	if count.max_line_chars > MAX_LINE_CHARS {
		report.problems.push(format!("A line has {} characters, at most {}", count.max_line_chars, MAX_LINE_CHARS));
	}
	match count.empty_lines {
		0 => (),
		1 => report.warnings.push("1 empty line: it is learned as an empty word".to_owned()),
		lines => report.warnings.push(format!("{} empty lines: each one is learned as an empty word", lines)),
	}
	match text.lines().filter(|line| line.contains(['<', '>'])).count() {
		0 => (),
		1 => report.warnings.push("1 line contains '<' or '>', which mark the start and the end of a word".to_owned()),
		lines => report.warnings.push(format!("{} lines contain '<' or '>', which mark the start and the end of a word", lines)),
	}

	if count.complete {
		let estimated = count.stats.estimated_bytes();
		let needed = count.stats.estimated_build_bytes(bytes.len() as u64);
		report.estimated_bytes = Some(estimated);
		report.needed_bytes = Some(needed);
		let needs = format!("building it needs about {} and keeping it {}", megabytes(needed), megabytes(estimated));
		match fit(needed, room) {
			Fit::InRam => (),
			Fit::WithSwap => report.risks.push(format!(
				"Not enough RAM: {}, the server has {} of RAM left (with a margin of {}). It fits by using swap, {} left with it: the generation will be slower",
				needs, megabytes_or_unknown(room.ram_bytes), megabytes(MEMORY_MARGIN_BYTES), megabytes_or_unknown(room.with_swap_bytes)
			)),
			Fit::TooBig => report.problems.push(match room.with_swap_bytes {
				Some(with_swap) => format!(
					"Not enough memory, even with swap: {}, the server has {} left with swap (with a margin of {})",
					needs, megabytes(with_swap), megabytes(MEMORY_MARGIN_BYTES)
				),
				None => format!(
					"Not enough memory: {}, the server has {} left (with a margin of {})",
					needs, megabytes_or_unknown(room.ram_bytes), megabytes(MEMORY_MARGIN_BYTES)
				),
			}),
			Fit::Unknown => report.warnings.push("The free memory of this system is unknown: the memory was not checked".to_owned()),
		}
	} else {
		report.problems.push(match (largest_room, room.with_swap_bytes) {
			(Some(largest), Some(_)) => format!("Not enough memory: the model has more than {} states, too many for the {} left with swap", count.stats.states, megabytes(largest)),
			(Some(largest), None) => format!("Not enough memory: the model has more than {} states, too many for the {} left", count.stats.states, megabytes(largest)),
			(None, _) => format!("The model has more than {} states, too many to be checked", count.stats.states),
		});
	}

	report.ok = report.problems.is_empty() && report.risks.is_empty();
	report
}

#[cfg(test)]
mod tests {
	use super::*;

	const GIGABYTE: u64 = 1_000_000_000;

	// Memory known in RAM only, the swap unknown
	fn ram(bytes: Option<u64>) -> Room {
		Room { ram_bytes: bytes, with_swap_bytes: None }
	}

	#[test]
	fn is_valid_name_accepts_the_existing_model_names() {
		for name in ["french", "elf-female", "us_tv_and_film", "rs-gen", "2025"] {
			assert!(is_valid_name(name), "{}", name);
		}
	}

	#[test]
	fn is_valid_name_rejects_paths_and_odd_names() {
		for name in ["", "../etc", "a/b", "a\\b", ".hidden", "-dash", "_under", "Upper", "été", "a b", "a.dat", &"a".repeat(65)] {
			assert!(!is_valid_name(name), "{}", name);
		}
	}

	#[test]
	fn a_small_word_list_passes() {
		let report = check_upload("fruits", false, "pomme\npoire\nprune\n".as_bytes(), ram(Some(GIGABYTE)));
		assert!(report.ok, "{:?}", report.problems);
		assert!(report.warnings.is_empty(), "{:?}", report.warnings);
		assert_eq!(report.count.unwrap().lines, 3);
		assert!(report.needed_bytes.unwrap() > report.estimated_bytes.unwrap());
	}

	#[test]
	fn a_taken_or_invalid_name_is_refused() {
		assert!(!check_upload("fruits", true, b"pomme\n", ram(Some(GIGABYTE))).ok);
		assert!(!check_upload("../fruits", false, b"pomme\n", ram(Some(GIGABYTE))).ok);
	}

	#[test]
	fn a_file_that_is_not_text_is_refused() {
		let report = check_upload("bytes", false, &[0xff, 0xfe, 0x00], ram(Some(GIGABYTE)));
		assert!(!report.ok);
		assert_eq!(report.count, None);
	}

	#[test]
	fn a_file_without_word_is_refused() {
		assert!(!check_upload("empty", false, b"", ram(Some(GIGABYTE))).ok);
		assert!(!check_upload("blank", false, b"\n\n", ram(Some(GIGABYTE))).ok);
	}

	#[test]
	fn a_too_long_line_is_refused() {
		let text = format!("short\n{}\n", "a".repeat(MAX_LINE_CHARS + 1));
		assert!(!check_upload("long", false, text.as_bytes(), ram(Some(GIGABYTE))).ok);
		let text = format!("short\n{}\n", "a".repeat(MAX_LINE_CHARS));
		assert!(check_upload("long", false, text.as_bytes(), ram(Some(GIGABYTE))).ok);
	}

	#[test]
	fn empty_lines_and_markers_are_warnings() {
		let report = check_upload("code", false, b"fn main\n\nVec<u8>\n", ram(Some(GIGABYTE)));
		assert!(report.ok, "{:?}", report.problems);
		assert_eq!(report.warnings, vec![
			"1 empty line: it is learned as an empty word",
			"1 line contains '<' or '>', which mark the start and the end of a word",
		]);
		let report = check_upload("code", false, b"a\n\n\n<b>\n<c\n", ram(Some(GIGABYTE)));
		assert_eq!(report.warnings, vec![
			"2 empty lines: each one is learned as an empty word",
			"2 lines contain '<' or '>', which mark the start and the end of a word",
		]);
	}

	#[test]
	fn check_label_keeps_a_human_name_and_refuses_control_characters() {
		assert_eq!(check_label("  Fromages de France  "), Ok(Some("Fromages de France".to_owned())));
		assert_eq!(check_label("Élfes & Fées"), Ok(Some("Élfes & Fées".to_owned())));
		assert_eq!(check_label("   "), Ok(None));
		assert_eq!(check_label(&"é".repeat(MAX_LABEL_CHARS)), Ok(Some("é".repeat(MAX_LABEL_CHARS))));
		assert!(check_label(&"é".repeat(MAX_LABEL_CHARS + 1)).is_err());
		assert!(check_label("two\nlines").is_err());
		assert!(check_label("a\ttab").is_err());
	}

	#[test]
	fn megabytes_reads_like_the_admin_page() {
		assert_eq!(megabytes(0), "0.00 MB");
		assert_eq!(megabytes(2_500), "< 0.01 MB");
		assert_eq!(megabytes(24_830_000), "24.83 MB");
	}

	#[test]
	fn a_model_bigger_than_the_free_memory_is_refused() {
		let text = "pomme\npoire\nprune\n";
		let needed = check_upload("fruits", false, text.as_bytes(), ram(Some(GIGABYTE))).needed_bytes.unwrap();
		let report = check_upload("fruits", false, text.as_bytes(), ram(Some(needed + MEMORY_MARGIN_BYTES - 1)));
		assert!(!report.ok);
		assert!(check_upload("fruits", false, text.as_bytes(), ram(Some(needed + MEMORY_MARGIN_BYTES))).ok);
	}

	#[test]
	fn counting_stops_when_the_model_cannot_fit() {
		// 1 kB left beyond the margin: the count stops after 16 states
		let report = check_upload("words", false, "abcdefgh\nijklmnop\n".as_bytes(), ram(Some(MEMORY_MARGIN_BYTES + 1024)));
		assert!(!report.ok);
		assert!(!report.count.unwrap().complete);
		assert_eq!(report.estimated_bytes, None);
	}

	#[test]
	fn an_unknown_free_memory_is_a_warning() {
		let report = check_upload("fruits", false, b"pomme\n", Room::default());
		assert!(report.ok, "{:?}", report.problems);
		assert_eq!(report.warnings.len(), 1);
	}

	#[test]
	fn fit_tells_ram_swap_or_too_big() {
		let room = Room { ram_bytes: Some(MEMORY_MARGIN_BYTES + 100), with_swap_bytes: Some(MEMORY_MARGIN_BYTES + 1000) };
		assert_eq!(fit(100, room), Fit::InRam);
		assert_eq!(fit(101, room), Fit::WithSwap);
		assert_eq!(fit(1000, room), Fit::WithSwap);
		assert_eq!(fit(1001, room), Fit::TooBig);
		assert_eq!(fit(101, ram(Some(MEMORY_MARGIN_BYTES + 100))), Fit::TooBig);
		assert_eq!(fit(1, Room::default()), Fit::Unknown);
	}

	#[test]
	fn a_model_that_fits_only_with_swap_is_a_risk() {
		let text = "pomme\npoire\nprune\n";
		let needed = check_upload("fruits", false, text.as_bytes(), ram(Some(GIGABYTE))).needed_bytes.unwrap();
		let room = Room { ram_bytes: Some(needed + MEMORY_MARGIN_BYTES - 1), with_swap_bytes: Some(GIGABYTE) };
		let report = check_upload("fruits", false, text.as_bytes(), room);
		assert!(!report.ok);
		assert!(report.problems.is_empty(), "{:?}", report.problems);
		assert_eq!(report.risks.len(), 1);
		assert!(report.risks[0].starts_with("Not enough RAM"), "{}", report.risks[0]);
		assert_eq!(report.available_with_swap_bytes, Some(GIGABYTE));
	}

	#[test]
	fn a_model_bigger_than_the_memory_with_swap_is_refused() {
		let text = "pomme\npoire\nprune\n";
		let needed = check_upload("fruits", false, text.as_bytes(), ram(Some(GIGABYTE))).needed_bytes.unwrap();
		let room = Room { ram_bytes: Some(MEMORY_MARGIN_BYTES), with_swap_bytes: Some(needed + MEMORY_MARGIN_BYTES - 1) };
		let report = check_upload("fruits", false, text.as_bytes(), room);
		assert!(!report.ok);
		assert!(report.risks.is_empty(), "{:?}", report.risks);
		assert_eq!(report.problems.len(), 1);
		assert!(report.problems[0].starts_with("Not enough memory, even with swap"), "{}", report.problems[0]);
	}

	#[test]
	fn a_risk_does_not_hide_a_problem() {
		// A taken name stays a problem even when the memory is only a risk
		let room = Room { ram_bytes: Some(MEMORY_MARGIN_BYTES), with_swap_bytes: Some(GIGABYTE) };
		let report = check_upload("fruits", true, b"pomme\n", room);
		assert!(!report.ok);
		assert_eq!(report.problems.len(), 1);
		assert_eq!(report.risks.len(), 1);
	}

	#[test]
	fn counting_goes_up_to_the_memory_with_swap() {
		// 1 kB of RAM beyond the margin, but a gigabyte with swap: the count is complete
		let room = Room { ram_bytes: Some(MEMORY_MARGIN_BYTES + 1024), with_swap_bytes: Some(GIGABYTE) };
		let report = check_upload("words", false, "abcdefgh\nijklmnop\n".as_bytes(), room);
		assert!(report.count.unwrap().complete);
		assert!(report.problems.is_empty(), "{:?}", report.problems);
		assert_eq!(report.risks.len(), 1);
	}
}
