use std::fs::File;
use std::{env, fs, io};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Reads a text file and returns all its lines as a `Vec<String>`.
///
/// # Parameters
/// - `filename`: Path to the file to read (implements `AsRef<Path>`).
///
/// # Returns
/// - `Ok(Vec<String>)`: a vector containing all the lines of the file.
/// - `Err(io::Error)`: if the file cannot be opened or read.
///
/// # Behavior
/// - Reads the entire file into memory before splitting it into lines.
/// - Lines are split on newline characters (`\n` or `\r\n`).
/// - Each line is cloned into the resulting vector.
pub fn read_file<P: AsRef<Path>>(filename: P) -> io::Result<Vec<String>> {
	let mut file = File::open(filename)?;
	let mut contents = String::new();
	let mut records = Vec::new();

	file.read_to_string(&mut contents)?;
	for line in contents.lines() {
		records.push(line.to_owned());
	}

	Ok(records)
}

/// Builds an output path based on an input path and a new extension.
///
/// # Parameters
/// - `input_path`: Path to the source file.
/// - `output_extension`: Desired extension for the output file.
///
/// # Returns
/// - `Ok(PathBuf)`: Full path to the output file with the new extension.
/// - `Err(io::Error)`: If the input path is invalid or has no file name.
///
/// # Behavior
/// - Retrieves the parent directory of the input file, defaults to `"."` if none.
/// - Gets the file name without its extension (`file_stem`).
/// - Creates a new `PathBuf` combining the parent directory and the file name.
/// - Sets the file extension to `output_extension`.
///
/// # Example
/// ```
/// let output = build_output_path("data/input.txt", "bin").unwrap();
/// assert_eq!(output.to_str().unwrap(), "data/input.bin");
/// ```
pub fn build_output_path<P: AsRef<Path>>(input_path: P, output_extension: &str) -> io::Result<PathBuf> {
	let input_filename = input_path.as_ref();
	let input_parent = input_filename.parent().unwrap_or_else(|| { Path::new(".") }); // Not sure
	let csv_file_prefix = input_filename.file_stem().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Input path has no file name"))?;

	let mut output_filename = PathBuf::from(input_parent);
	output_filename.push(csv_file_prefix);
	output_filename.set_extension(output_extension);

	Ok(output_filename)
}

/// Extracts the **base file name without extension** from a path.
///
/// This function returns only the file stem:
/// - `"./data/model.dat"` → `"model"`
/// - `"model.dat"` → `"model"`
///
/// Directory components and file extensions are removed.
///
/// # Type Parameters
///
/// - `P`: Any type that can be referenced as a [`Path`]
///
/// # Arguments
///
/// - `input_path`: Path to a file
///
/// # Returns
///
/// - `Ok(String)` containing the file name without extension
/// - `Err(io::Error)` if the path has no valid file name
///
/// # Errors
///
/// This function returns an error if:
/// - The path does not contain a file name
/// - The file name cannot be extracted
///
/// # Examples
///
/// ```rust
/// let name = get_filename("./data/example.dat")?;
/// assert_eq!(name, "example");
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn get_filename<P: AsRef<Path>>(input_path: P) -> io::Result<String> {
	let path = input_path.as_ref();

	let stem = path
		.file_stem()
		.ok_or_else(|| {
			io::Error::new(io::ErrorKind::InvalidInput, "Path has no filename")
		})?;

	Ok(stem.to_string_lossy().to_string())
}

/// Normalize a folder path.
/// - If the user passes "./" → resolves to the current working directory.
/// - Returns a PathBuf.
pub fn resolve_folder(input: &str) -> PathBuf {
	let path = Path::new(input);
	if input == "./" || input == "." {
		env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
	} else {
		path.to_path_buf()
	}
}

/// Lists **all entries** (files and directories) contained in a given folder.
///
/// This function reads the directory located at `folder` and returns a
/// **sorted list of paths** corresponding to all entries inside it.
/// Both files and subdirectories are included.
///
/// # Type Parameters
///
/// - `P`: Any type that can be referenced as a [`Path`]
///
/// # Arguments
///
/// - `folder`: Path to the directory to list
///
/// # Returns
///
/// - `Ok(Vec<PathBuf>)` containing all directory entries, sorted lexicographically
/// - `Err(io::Error)` if the directory cannot be read
///
/// # Errors
///
/// This function will return an error if:
/// - The path does not exist
/// - The path is not a directory
/// - The process lacks read permissions
///
/// # Examples
///
/// ```rust,no_run
/// let entries = list_all_entries("./data")?;
/// for entry in entries {
///     println!("{}", entry.display());
/// }
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn list_all_entries<P: AsRef<Path>>(folder: &P) -> io::Result<Vec<PathBuf>> {
	let mut entries: Vec<_> = fs::read_dir(folder)?
		.filter_map(|entry| entry.ok())
		.map(|entry| entry.path())
		.collect();
	entries.sort();
	Ok(entries)
}

/// Lists all files with a given extension in a directory.
///
/// This function scans the specified directory and returns the **file names**
/// (not full paths) of all files whose extension matches `extension`.
///
/// Subdirectories are ignored.
///
/// # Type Parameters
///
/// - `P`: Any type that can be referenced as a [`Path`]
///
/// # Arguments
///
/// - `dir`: Path to the directory to scan
/// - `extension`: File extension to match (without the dot), e.g. `"dat"`
///
/// # Returns
///
/// - `Ok(Vec<String>)` containing the matching file names
/// - `Err(io::Error)` if the directory cannot be read
///
/// # Errors
///
/// This function returns an error if:
/// - The directory does not exist
/// - The directory cannot be read due to permissions or IO failure
///
/// # Examples
///
/// ```rust,no_run
/// let files = list_files("./data", "dat")?;
/// for file in files {
///     println!("{file}");
/// }
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn list_files<P: AsRef<Path>>(dir: &P, extension: &str) -> io::Result<Vec<String>> {
	let mut files = Vec::new();

	for entry in fs::read_dir(dir)? {
		let entry = entry?;
		let path = entry.path();

		if path.is_file() {
			if let Some(ext) = path.extension() {
				if ext == extension {
					if let Some(name) = path.file_name() {
						files.push(name.to_string_lossy().to_string());
					}
				}
			}
		}
	}

	Ok(files)
}
