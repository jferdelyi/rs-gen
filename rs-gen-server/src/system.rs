//! Memory of the server process and of the machine, read from `/proc` and the cgroup of the service.
//!
//! Linux only: on another system every value is unknown (`None`).

use serde::Serialize;

/// Memory figures shown on the admin page and used to check whether a model fits.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct MemoryInfo {
	/// Memory of the server process kept in RAM.
	pub(crate) process_resident_bytes: Option<u64>,

	/// Memory of the server process moved to the swap.
	pub(crate) process_swap_bytes: Option<u64>,

	/// Memory the machine can still give without swapping (`MemAvailable`).
	pub(crate) system_available_bytes: Option<u64>,

	/// Memory limit of the service (`MemoryMax` of systemd), if any.
	pub(crate) service_limit_bytes: Option<u64>,

	/// Memory used by the service, counted against its limit.
	pub(crate) service_used_bytes: Option<u64>,
}

impl MemoryInfo {
	/// Reads the current figures; the unreadable ones stay `None`.
	pub(crate) fn read() -> Self {
		let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
		let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
		let mut info = Self {
			process_resident_bytes: kib_field(&status, "VmRSS:"),
			process_swap_bytes: kib_field(&status, "VmSwap:"),
			system_available_bytes: kib_field(&meminfo, "MemAvailable:"),
			..Self::default()
		};
		if let Some(dir) = std::fs::read_to_string("/proc/self/cgroup").ok().and_then(|text| cgroup_dir(&text)) {
			info.service_limit_bytes = std::fs::read_to_string(format!("{}/memory.max", dir)).ok().and_then(|text| limit_value(&text));
			info.service_used_bytes = std::fs::read_to_string(format!("{}/memory.current", dir)).ok().and_then(|text| text.trim().parse().ok());
		}
		info
	}

	/// Memory that a new model can still take: what the machine has left,
	/// and no more than what the service limit leaves. `None` if unknown.
	pub(crate) fn available_bytes(&self) -> Option<u64> {
		let service = match (self.service_limit_bytes, self.service_used_bytes) {
			(Some(limit), Some(used)) => Some(limit.saturating_sub(used)),
			_ => None,
		};
		match (self.system_available_bytes, service) {
			(Some(system), Some(service)) => Some(system.min(service)),
			(system, service) => system.or(service),
		}
	}
}

/// Reads a `<field> <value> kB` line of `/proc/self/status` or `/proc/meminfo`, in bytes.
fn kib_field(text: &str, field: &str) -> Option<u64> {
	let line = text.lines().find(|line| line.starts_with(field))?;
	let kib: u64 = line[field.len()..].split_whitespace().next()?.parse().ok()?;
	Some(kib * 1024)
}

/// Returns the directory of the cgroup v2 of the process, from `/proc/self/cgroup`.
fn cgroup_dir(text: &str) -> Option<String> {
	let path = text.lines().find_map(|line| line.strip_prefix("0::"))?;
	Some(format!("/sys/fs/cgroup{}", path.trim()))
}

/// Reads a `memory.max` value: a number of bytes, or `max` for no limit.
fn limit_value(text: &str) -> Option<u64> {
	text.trim().parse().ok()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn kib_field_reads_a_value_in_bytes() {
		let status = "Name:\trs-gen-server\nVmRSS:\t  140908 kB\nVmSwap:\t  427356 kB\n";
		assert_eq!(kib_field(status, "VmRSS:"), Some(140_908 * 1024));
		assert_eq!(kib_field(status, "VmSwap:"), Some(427_356 * 1024));
		assert_eq!(kib_field(status, "VmData:"), None);
	}

	#[test]
	fn cgroup_dir_reads_the_cgroup_v2_path() {
		assert_eq!(cgroup_dir("0::/system.slice/generator.service\n"), Some("/sys/fs/cgroup/system.slice/generator.service".to_owned()));
		assert_eq!(cgroup_dir("12:memory:/old\n"), None);
	}

	#[test]
	fn limit_value_reads_no_limit_as_none() {
		assert_eq!(limit_value("max\n"), None);
		assert_eq!(limit_value("734003200\n"), Some(734_003_200));
	}

	#[test]
	fn available_bytes_keeps_the_lowest_bound() {
		let mut info = MemoryInfo { system_available_bytes: Some(300), ..MemoryInfo::default() };
		assert_eq!(info.available_bytes(), Some(300));
		info.service_limit_bytes = Some(700);
		info.service_used_bytes = Some(600);
		assert_eq!(info.available_bytes(), Some(100));
		info.service_used_bytes = Some(800);
		assert_eq!(info.available_bytes(), Some(0));
		assert_eq!(MemoryInfo::default().available_bytes(), None);
	}
}
