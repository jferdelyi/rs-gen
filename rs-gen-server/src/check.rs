//! Checks a text file sent by the admin before it becomes a model: name, content, and
//! whether the server has the memory to build it and keep it.

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

/// Every state costs more than this in the memory estimate: counting stops beyond the
/// free memory divided by it, since such a model cannot fit.
const MIN_BYTES_PER_STATE: u64 = 64;

/// Longest model name.
const MAX_NAME_CHARS: usize = 64;

/// Result of a check, sent to the admin page.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckReport {
	/// Name of the model.
	pub(crate) name: String,

	/// True when nothing prevents adding the model.
	pub(crate) ok: bool,

	/// What prevents adding the model.
	pub(crate) problems: Vec<String>,

	/// What does not prevent adding the model but deserves a look.
	pub(crate) warnings: Vec<String>,

	/// What the text would give, if it could be read.
	pub(crate) count: Option<TextCount>,

	/// Estimated memory of the model once loaded, in bytes.
	pub(crate) estimated_bytes: Option<u64>,

	/// Estimated peak memory while building it, in bytes.
	pub(crate) needed_bytes: Option<u64>,

	/// Memory the server can still give, in bytes, if known.
	pub(crate) available_bytes: Option<u64>,
}

/// Tells whether `name` can name a model: 1 to 64 lowercase letters, digits, `_` or `-`,
/// starting with a letter or a digit. Such a name is also a safe file name.
pub(crate) fn is_valid_name(name: &str) -> bool {
	let mut chars = name.chars();
	chars.next().is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
		&& name.len() <= MAX_NAME_CHARS
		&& name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
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
/// - `available_bytes`: memory the server can still give, `None` if unknown.
pub(crate) fn check_upload(name: &str, name_taken: bool, bytes: &[u8], available_bytes: Option<u64>) -> CheckReport {
	let mut report = CheckReport {
		name: name.to_owned(),
		ok: false,
		problems: Vec::new(),
		warnings: Vec::new(),
		count: None,
		estimated_bytes: None,
		needed_bytes: None,
		available_bytes,
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

	// Counting stops as soon as the model cannot fit, so a huge file does not exhaust the memory
	let max_states = match available_bytes {
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
		match available_bytes {
			Some(available) if needed.saturating_add(MEMORY_MARGIN_BYTES) > available => report.problems.push(format!(
				"Not enough memory: building it needs about {} and keeping it {}, the server has {} left (with a margin of {})",
				megabytes(needed), megabytes(estimated), megabytes(available), megabytes(MEMORY_MARGIN_BYTES)
			)),
			Some(_) => (),
			None => report.warnings.push("The free memory of this system is unknown: the memory was not checked".to_owned()),
		}
	} else {
		report.problems.push(match available_bytes {
			Some(available) => format!("Not enough memory: the model has more than {} states, too many for the {} left", count.stats.states, megabytes(available)),
			None => format!("The model has more than {} states, too many to be checked", count.stats.states),
		});
	}

	report.ok = report.problems.is_empty();
	report
}

#[cfg(test)]
mod tests {
	use super::*;

	const GIGABYTE: u64 = 1_000_000_000;

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
		let report = check_upload("fruits", false, "pomme\npoire\nprune\n".as_bytes(), Some(GIGABYTE));
		assert!(report.ok, "{:?}", report.problems);
		assert!(report.warnings.is_empty(), "{:?}", report.warnings);
		assert_eq!(report.count.unwrap().lines, 3);
		assert!(report.needed_bytes.unwrap() > report.estimated_bytes.unwrap());
	}

	#[test]
	fn a_taken_or_invalid_name_is_refused() {
		assert!(!check_upload("fruits", true, b"pomme\n", Some(GIGABYTE)).ok);
		assert!(!check_upload("../fruits", false, b"pomme\n", Some(GIGABYTE)).ok);
	}

	#[test]
	fn a_file_that_is_not_text_is_refused() {
		let report = check_upload("bytes", false, &[0xff, 0xfe, 0x00], Some(GIGABYTE));
		assert!(!report.ok);
		assert_eq!(report.count, None);
	}

	#[test]
	fn a_file_without_word_is_refused() {
		assert!(!check_upload("empty", false, b"", Some(GIGABYTE)).ok);
		assert!(!check_upload("blank", false, b"\n\n", Some(GIGABYTE)).ok);
	}

	#[test]
	fn a_too_long_line_is_refused() {
		let text = format!("short\n{}\n", "a".repeat(MAX_LINE_CHARS + 1));
		assert!(!check_upload("long", false, text.as_bytes(), Some(GIGABYTE)).ok);
		let text = format!("short\n{}\n", "a".repeat(MAX_LINE_CHARS));
		assert!(check_upload("long", false, text.as_bytes(), Some(GIGABYTE)).ok);
	}

	#[test]
	fn empty_lines_and_markers_are_warnings() {
		let report = check_upload("code", false, b"fn main\n\nVec<u8>\n", Some(GIGABYTE));
		assert!(report.ok, "{:?}", report.problems);
		assert_eq!(report.warnings, vec![
			"1 empty line: it is learned as an empty word",
			"1 line contains '<' or '>', which mark the start and the end of a word",
		]);
		let report = check_upload("code", false, b"a\n\n\n<b>\n<c\n", Some(GIGABYTE));
		assert_eq!(report.warnings, vec![
			"2 empty lines: each one is learned as an empty word",
			"2 lines contain '<' or '>', which mark the start and the end of a word",
		]);
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
		let needed = check_upload("fruits", false, text.as_bytes(), Some(GIGABYTE)).needed_bytes.unwrap();
		let report = check_upload("fruits", false, text.as_bytes(), Some(needed + MEMORY_MARGIN_BYTES - 1));
		assert!(!report.ok);
		assert!(check_upload("fruits", false, text.as_bytes(), Some(needed + MEMORY_MARGIN_BYTES)).ok);
	}

	#[test]
	fn counting_stops_when_the_model_cannot_fit() {
		// 1 kB left beyond the margin: the count stops after 16 states
		let report = check_upload("words", false, "abcdefgh\nijklmnop\n".as_bytes(), Some(MEMORY_MARGIN_BYTES + 1024));
		assert!(!report.ok);
		assert!(!report.count.unwrap().complete);
		assert_eq!(report.estimated_bytes, None);
	}

	#[test]
	fn an_unknown_free_memory_is_a_warning() {
		let report = check_upload("fruits", false, b"pomme\n", None);
		assert!(report.ok, "{:?}", report.problems);
		assert_eq!(report.warnings.len(), 1);
	}
}
