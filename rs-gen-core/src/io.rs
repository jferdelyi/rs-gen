use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

/// Reads a text file and returns all its lines as a `Vec<String>`.
///
/// - Reads the entire file into memory
/// - Splits on `\n` / `\r\n`
pub(crate) fn read_file<P: AsRef<Path>>(filename: P) -> io::Result<Vec<String>> {
	let mut contents = String::new();
	File::open(filename)?.read_to_string(&mut contents)?;
	Ok(contents.lines().map(str::to_owned).collect())
}

/// Builds an output path based on an input path and a new extension.
///
/// Example:
/// `data/input.txt` + `"bin"` → `data/input.bin`
pub(crate) fn build_output_path<P: AsRef<Path>>(
	input_path: P,
	output_extension: &str,
) -> io::Result<PathBuf> {
	let input_path = input_path.as_ref();

	let parent = input_path.parent().unwrap_or_else(|| Path::new("."));
	let file_stem = input_path
		.file_stem()
		.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Input path has no filename"))?;

	let mut output = PathBuf::from(parent);
	output.push(file_stem);
	output.set_extension(output_extension);

	Ok(output)
}

/// Extracts the base filename without extension.
///
/// Examples:
/// - `"./data/model.dat"` → `"model"`
/// - `"model.dat"` → `"model"`
pub(crate) fn get_filename<P: AsRef<Path>>(input_path: P) -> io::Result<String> {
	let stem = input_path
		.as_ref()
		.file_stem()
		.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Path has no filename"))?;

	Ok(stem.to_string_lossy().to_string())
}

/// Normalize a folder path.
///
/// - `"."` or `"./"` resolves to the current working directory
/// - Other paths are returned as-is (not canonicalized)
pub(crate) fn normalize_folder(input: &str) -> PathBuf {
	if input == "." || input == "./" {
		env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
	} else {
		PathBuf::from(input)
	}
}

/// Lists all files with a given extension in a directory.
///
/// Returns file names only (no paths).
pub(crate) fn list_files<P: AsRef<Path>>(dir: P, extension: &str) -> io::Result<Vec<String>> {
	let mut files = Vec::new();

	for entry in fs::read_dir(dir)? {
		let entry = entry?;
		let path = entry.path();

		if path.is_file() {
			if path.extension() == Some(std::ffi::OsStr::new(extension)) {
				if let Some(name) = path.file_name() {
					files.push(name.to_string_lossy().to_string());
				}
			}
		}
	}

	Ok(files)
}
