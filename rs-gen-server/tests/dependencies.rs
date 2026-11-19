//! Security fixes of the locked dependencies, checked on `Cargo.lock`.
//!
//! The HTTP server is reachable from the Internet through the reverse proxy: a dependency locked
//! back to a version with a known vulnerability (RustSec advisory database) must fail the tests.

use std::path::PathBuf;

/// Lowest version of each crate that has the fix, with the advisories it closes.
const FIXED: &[(&str, &str)] = &[
	// RUSTSEC-2026-0446, 0448, 0455, 0552, 0553, 0554: request smuggling, decompression, timeouts
	("actix-http", "3.21.3"),
	// RUSTSEC-2026-0447: compressed request bodies exhaust memory before the extractor limits
	("actix-web", "4.18.2"),
	// RUSTSEC-2026-0007: integer overflow in BytesMut::reserve
	("bytes", "1.11.1"),
	// RUSTSEC-2026-0009: denial of service through stack exhaustion
	("time", "0.3.47"),
	// RUSTSEC-2026-0391: data of an abandoned stream leaks into the next one
	("zstd", "0.13.4"),
	// RUSTSEC-2026-0398, 0403 and the unsound zstd-safe 7.2 advisories
	("zstd-safe", "7.4.0"),
	// RUSTSEC-2026-0410: memory corruption on allocation failure
	("zstd-sys", "2.1.2"),
	// RUSTSEC-2026-0097: unsound with a custom logger
	("rand", "0.9.5"),
];

/// The packages of `Cargo.lock`: name and version.
fn locked() -> Vec<(String, String)> {
	let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "Cargo.lock"].iter().collect();
	let lock = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e));
	let mut packages = Vec::new();
	let mut name = None;
	for line in lock.lines() {
		if line == "[[package]]" {
			name = None;
		} else if let Some(value) = line.strip_prefix("name = ") {
			name = Some(value.trim_matches('"').to_string());
		} else if let (Some(value), Some(current)) = (line.strip_prefix("version = "), name.take()) {
			packages.push((current, value.trim_matches('"').to_string()));
		}
	}
	packages
}

/// The numbers of a version, its build metadata (`+zstd.1.5.7`) left out.
fn numbers(version: &str) -> Vec<u64> {
	let core = version.split(['+', '-']).next().unwrap_or(version);
	core.split('.').map(|n| n.parse().unwrap_or_else(|_| panic!("not a version: {version}"))).collect()
}

#[test]
fn every_locked_dependency_has_its_security_fixes() {
	let packages = locked();
	let mut old = Vec::new();
	for (crate_name, fixed) in FIXED {
		for (name, version) in packages.iter().filter(|(name, _)| name == crate_name) {
			if numbers(version) < numbers(fixed) {
				old.push(format!("{name} {version}, fixed in {fixed}"));
			}
		}
	}
	assert!(old.is_empty(), "versions with known vulnerabilities in Cargo.lock:\n{}", old.join("\n"));
}

#[test]
fn the_lock_file_is_read() {
	// Positive control: without it, a check that finds no old version would prove nothing
	let packages = locked();
	assert!(packages.len() > 100, "only {} packages read in Cargo.lock", packages.len());
	for (crate_name, _) in FIXED {
		assert!(packages.iter().any(|(name, _)| name == crate_name), "{crate_name} is not in Cargo.lock");
	}
	assert!(numbers("2.1.3+zstd.1.5.7") == [2, 1, 3]);
	assert!(numbers("0.9.5") < numbers("0.10.4"));
}
