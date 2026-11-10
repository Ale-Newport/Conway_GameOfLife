//! Description of the machine a benchmark ran on.

use std::process::Command;

use serde::Serialize;

/// Hardware and build facts printed with benchmark results.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MachineInfo {
    /// CPU model as reported by the operating system.
    pub cpu: Option<String>,
    /// Logical CPUs available to this process.
    pub logical_cores: usize,
    /// Physical cores (macOS and Linux).
    pub physical_cores: Option<usize>,
    /// Installed memory in bytes (macOS and Linux).
    pub memory_bytes: Option<u64>,
    /// Operating system family and version.
    pub os: String,
    /// CPU architecture.
    pub arch: String,
    /// Threads in the global rayon pool used by parallel models.
    pub rayon_threads: usize,
    /// Whether debug assertions were compiled in (a debug-profile build).
    pub debug_assertions: bool,
}

/// First line of a command's standard output.
fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Value of the first `key : value` line of a `/proc` style file.
fn proc_value(path: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|line| {
        let (k, v) = line.split_once(':').or_else(|| line.split_once('='))?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

impl MachineInfo {
    /// Collects the information of the current machine.
    pub fn collect() -> Self {
        let logical_cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let (cpu, physical_cores, memory_bytes, os_version) = match std::env::consts::OS {
            "macos" => (
                command_line("sysctl", &["-n", "machdep.cpu.brand_string"]),
                command_line("sysctl", &["-n", "hw.physicalcpu"]).and_then(|v| v.parse().ok()),
                command_line("sysctl", &["-n", "hw.memsize"]).and_then(|v| v.parse().ok()),
                command_line("sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}")),
            ),
            "linux" => (
                proc_value("/proc/cpuinfo", "model name"),
                proc_value("/proc/cpuinfo", "cpu cores").and_then(|v| v.parse().ok()),
                proc_value("/proc/meminfo", "MemTotal")
                    .and_then(|v| v.trim_end_matches("kB").trim().parse::<u64>().ok())
                    .map(|kib| kib * 1024),
                proc_value("/etc/os-release", "PRETTY_NAME"),
            ),
            "windows" => (std::env::var("PROCESSOR_IDENTIFIER").ok(), None, None, None),
            _ => (None, None, None, None),
        };
        Self {
            cpu,
            logical_cores,
            physical_cores,
            memory_bytes,
            os: os_version.unwrap_or_else(|| std::env::consts::OS.to_string()),
            arch: std::env::consts::ARCH.to_string(),
            rayon_threads: rayon::current_num_threads(),
            debug_assertions: cfg!(debug_assertions),
        }
    }
}
