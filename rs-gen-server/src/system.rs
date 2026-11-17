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

	/// Swap the machine has left (`SwapFree`).
	pub(crate) system_swap_free_bytes: Option<u64>,

	/// Swap limit of the service (`MemorySwapMax` of systemd), if any.
	pub(crate) service_swap_limit_bytes: Option<u64>,

	/// Swap used by the service, counted against its swap limit.
	pub(crate) service_swap_used_bytes: Option<u64>,
}

/// Memory that a new model can take, in bytes: in RAM only, and in RAM plus swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Room {
	/// What the model can take without swapping, `None` if unknown.
	pub(crate) ram_bytes: Option<u64>,

	/// What the model can take by using swap too, `None` if unknown.
	pub(crate) with_swap_bytes: Option<u64>,
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
			system_swap_free_bytes: kib_field(&meminfo, "SwapFree:"),
			..Self::default()
		};
		if let Some(dir) = std::fs::read_to_string("/proc/self/cgroup").ok().and_then(|text| cgroup_dir(&text)) {
			let read = |file: &str| std::fs::read_to_string(format!("{}/{}", dir, file)).ok();
			info.service_limit_bytes = read("memory.max").and_then(|text| limit_value(&text));
			info.service_used_bytes = read("memory.current").and_then(|text| text.trim().parse().ok());
			info.service_swap_limit_bytes = read("memory.swap.max").and_then(|text| limit_value(&text));
			info.service_swap_used_bytes = read("memory.swap.current").and_then(|text| text.trim().parse().ok());
		}
		info
	}

	/// Memory that a new model can take: in RAM, and in RAM plus swap.
	pub(crate) fn room(&self) -> Room {
		Room { ram_bytes: self.available_bytes(), with_swap_bytes: self.available_with_swap_bytes() }
	}

	/// Memory that a new model can still take: what the machine has left,
	/// and no more than what the service limit leaves. `None` if unknown.
	pub(crate) fn available_bytes(&self) -> Option<u64> {
		let service = match (self.service_limit_bytes, self.service_used_bytes) {
			(Some(limit), Some(used)) => Some(limit.saturating_sub(used)),
			_ => None,
		};
		lowest(self.system_available_bytes, service)
	}

	/// Memory that a new model can take by using swap too: what the machine has left
	/// in RAM and in swap, and no more than what the service limits leave in both.
	/// A service without swap limit is bounded by the machine only. Without the free swap
	/// of the machine, nothing more than in RAM. `None` if unknown.
	pub(crate) fn available_with_swap_bytes(&self) -> Option<u64> {
		let Some(swap_free) = self.system_swap_free_bytes else {
			return self.available_bytes();
		};
		let system = self.system_available_bytes.map(|ram| ram.saturating_add(swap_free));
		let service = match (self.service_limit_bytes, self.service_used_bytes, self.service_swap_limit_bytes, self.service_swap_used_bytes) {
			(Some(limit), Some(used), Some(swap_limit), Some(swap_used)) => {
				Some(limit.saturating_sub(used).saturating_add(swap_limit.saturating_sub(swap_used)))
			}
			_ => None,
		};
		lowest(system, service)
	}
}

/// The lowest of two bounds, ignoring the unknown one.
fn lowest(a: Option<u64>, b: Option<u64>) -> Option<u64> {
	match (a, b) {
		(Some(a), Some(b)) => Some(a.min(b)),
		(a, b) => a.or(b),
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

	#[test]
	fn available_with_swap_bytes_adds_the_swap_within_the_limits() {
		// Figures of the VPS, in MB: RAM nearly full, swap mostly free
		let mut info = MemoryInfo { system_available_bytes: Some(90), system_swap_free_bytes: Some(1900), ..MemoryInfo::default() };
		assert_eq!(info.available_with_swap_bytes(), Some(1990));
		// MemoryMax=700M and MemorySwapMax=1G: the service limits win
		info.service_limit_bytes = Some(700);
		info.service_used_bytes = Some(610);
		info.service_swap_limit_bytes = Some(1000);
		info.service_swap_used_bytes = Some(100);
		assert_eq!(info.available_bytes(), Some(90));
		assert_eq!(info.available_with_swap_bytes(), Some(990));
		assert_eq!(info.room(), Room { ram_bytes: Some(90), with_swap_bytes: Some(990) });
		// No swap allowed to the service: nothing more than in RAM
		info.service_swap_limit_bytes = Some(0);
		info.service_swap_used_bytes = Some(0);
		assert_eq!(info.available_with_swap_bytes(), Some(90));
		// No swap limit (`max`): the machine bounds it
		info.service_swap_limit_bytes = None;
		assert_eq!(info.available_with_swap_bytes(), Some(1990));
		// Swap of the machine unknown: RAM only, the service RAM limit included
		info.system_swap_free_bytes = None;
		info.service_used_bytes = Some(650);
		assert_eq!(info.available_with_swap_bytes(), Some(50));
		assert_eq!(MemoryInfo::default().room(), Room::default());
	}
}
