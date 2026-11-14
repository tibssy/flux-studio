use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::paths;

/// Default manifest location. The bundled copy is used when this cannot be
/// fetched or when the app is offline.
pub const DEFAULT_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/tibssy/flux-studio/main/backends/manifest.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    Cpu,
    Vulkan,
    Rocm,
    Cuda,
}

impl BackendKind {
    pub fn label(self) -> &'static str {
        match self {
            BackendKind::Cpu => "CPU",
            BackendKind::Vulkan => "Vulkan",
            BackendKind::Rocm => "ROCm",
            BackendKind::Cuda => "CUDA",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendEntry {
    pub id: String,
    pub label: String,
    pub kind: BackendKind,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub cuda_major: Option<u32>,
    #[serde(default)]
    pub rocm_major: Option<u32>,
    #[serde(default)]
    pub archs: Vec<u32>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub min_glibc: Option<String>,
    #[serde(default)]
    pub strip_prefix: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl BackendEntry {
    pub fn install_dir(&self, root: &Path) -> PathBuf {
        root.join(&self.id)
    }

    pub fn server_path(&self, root: &Path) -> PathBuf {
        self.install_dir(root).join("sd-server")
    }

    pub fn marker_path(&self, root: &Path) -> PathBuf {
        self.install_dir(root).join("installed.json")
    }

    pub fn is_installed(&self, root: &Path) -> bool {
        self.server_path(root).is_file()
    }

    pub fn installed_version(&self, root: &Path) -> Option<String> {
        let text = std::fs::read_to_string(self.marker_path(root)).ok()?;
        let marker: InstalledBackend = serde_json::from_str(&text).ok()?;
        Some(marker.version)
    }

    pub fn is_available(&self) -> bool {
        !self.url.is_empty() && self.size > 0 && !self.sha256.is_empty()
    }

    /// Whether this build supports the detected GPU architectures.
    pub fn supports_archs(&self, compute_cap: Option<u32>) -> bool {
        if self.archs.is_empty() {
            return true;
        }
        compute_cap.is_some_and(|cap| self.archs.contains(&cap))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledBackend {
    pub id: String,
    pub version: String,
    pub kind: BackendKind,
    pub installed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendManifest {
    pub schema_version: u32,
    pub generated_at: String,
    pub backends: Vec<BackendEntry>,
}

impl BackendManifest {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let manifest: Self = serde_json::from_str(text)?;
        anyhow::ensure!(
            manifest.schema_version == 1,
            "unsupported backend manifest schema {}",
            manifest.schema_version
        );
        Ok(manifest)
    }

    pub fn entry(&self, id: &str) -> Option<&BackendEntry> {
        self.backends.iter().find(|entry| entry.id == id)
    }

    pub fn cpu_entry(&self) -> Option<&BackendEntry> {
        self.backends
            .iter()
            .find(|entry| entry.kind == BackendKind::Cpu && entry.is_available())
    }
}

pub fn bundled_manifest() -> BackendManifest {
    BackendManifest::parse(include_str!("../../../backends/manifest.json"))
        .expect("bundled backend manifest is valid")
}

#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub name: String,
    pub compute_cap: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct HardwareProfile {
    pub nvidia: Option<GpuInfo>,
    pub amd: bool,
    pub vulkan: bool,
    pub cuda_majors: Vec<u32>,
    pub glibc: Option<(u32, u32)>,
}

impl HardwareProfile {
    pub fn detect() -> Self {
        Self {
            nvidia: detect_nvidia(),
            amd: detect_amd(),
            vulkan: has_library("libvulkan.so.1"),
            cuda_majors: detect_cuda_majors(),
            glibc: detect_glibc(),
        }
    }

    pub fn compatibility(&self, entry: &BackendEntry) -> Option<String> {
        if let (Some(min), Some(current)) = (entry.min_glibc.as_deref(), self.glibc)
            && let Some(required) = parse_version(min)
            && current < required
        {
            return Some(format!(
                "Requires glibc {min} (this system has {}.{})",
                current.0, current.1
            ));
        }

        match entry.kind {
            BackendKind::Cpu => None,
            BackendKind::Vulkan => {
                if self.vulkan {
                    None
                } else {
                    Some("Vulkan loader (libvulkan.so.1) not found".into())
                }
            }
            BackendKind::Rocm => {
                if self.amd {
                    None
                } else {
                    Some("No AMD GPU detected".into())
                }
            }
            BackendKind::Cuda => {
                let Some(gpu) = &self.nvidia else {
                    return Some("No NVIDIA GPU detected".into());
                };
                if let Some(major) = entry.cuda_major
                    && !self.cuda_majors.contains(&major)
                {
                    let found = if self.cuda_majors.is_empty() {
                        "none".to_string()
                    } else {
                        self.cuda_majors
                            .iter()
                            .map(|major| format!("CUDA {major}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    return Some(format!("CUDA {major} runtime not found (found: {found})"));
                }
                if !entry.supports_archs(gpu.compute_cap) {
                    return Some(format!(
                        "This build does not include sm_{} (architectures: {})",
                        gpu.compute_cap.unwrap_or(0),
                        entry
                            .archs
                            .iter()
                            .map(|arch| format!("sm_{arch}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                None
            }
        }
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(gpu) = &self.nvidia {
            parts.push(format!(
                "NVIDIA {} (sm_{})",
                gpu.name,
                gpu.compute_cap.unwrap_or(0)
            ));
        }
        if self.amd {
            parts.push("AMD GPU".into());
        }
        if self.vulkan {
            parts.push("Vulkan".into());
        }
        if !self.cuda_majors.is_empty() {
            parts.push(format!(
                "CUDA {}",
                self.cuda_majors
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join("/")
            ));
        }
        parts.push("CPU".into());
        parts.join(" · ")
    }
}

/// Places a user-installed server is commonly found.
pub fn system_server_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("sd-server"));
        candidates.push(dir.join("backends").join("sd-server"));
        candidates.push(dir.join("../lib/flux-studio/sd-server"));
        candidates.push(dir.join("../libexec/flux-studio/sd-server"));
    }
    candidates.push(PathBuf::from("/usr/lib/flux-studio/sd-server"));
    candidates.push(PathBuf::from("/usr/lib/sd-server"));
    candidates.push(PathBuf::from("/usr/lib64/sd-server"));
    candidates.push(PathBuf::from("/usr/libexec/sd-server"));
    candidates.push(PathBuf::from("/usr/local/lib/flux-studio/sd-server"));
    candidates.push(PathBuf::from("/usr/local/bin/sd-server"));
    candidates.push(PathBuf::from("/usr/bin/sd-server"));
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/sd-server"));
        candidates.push(home.join(".local/lib/flux-studio/sd-server"));
    }
    candidates
}

pub fn find_system_server() -> Option<PathBuf> {
    system_server_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| find_in_path("sd-server"))
}

/// Check that a server binary can at least load its shared libraries.
pub fn server_works(path: &Path) -> bool {
    path.is_file()
        && std::process::Command::new(path)
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
}

pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn backends_dir() -> PathBuf {
    paths::backends_dir()
}

fn detect_nvidia() -> Option<GpuInfo> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=name,compute_cap", "--format=csv,noheader"])
        .output()
        .ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(line) = text
            .lines()
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let (name, cap) = line
                .split_once(',')
                .map(|(name, cap)| (name.trim(), parse_compute_cap(cap.trim())))
                .unwrap_or((line, None));
            return Some(GpuInfo {
                name: name.to_string(),
                compute_cap: cap,
            });
        }
    }
    // Older drivers do not support compute_cap in --query-gpu.
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    (!name.is_empty()).then_some(GpuInfo {
        name,
        compute_cap: None,
    })
}

fn detect_amd() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("card") {
            continue;
        }
        let vendor = entry.path().join("device/vendor");
        if let Ok(value) = std::fs::read_to_string(vendor)
            && value.trim().eq_ignore_ascii_case("0x1002")
        {
            return true;
        }
    }
    false
}

fn detect_cuda_majors() -> Vec<u32> {
    let mut majors = Vec::new();
    if let Some(text) = ldconfig_output() {
        for line in text.lines() {
            if let Some(index) = line.find("libcudart.so.") {
                let rest = &line[index + "libcudart.so.".len()..];
                let major: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(major) = major.parse::<u32>()
                    && !majors.contains(&major)
                {
                    majors.push(major);
                }
            }
        }
    }
    for pattern in [
        "/usr/local",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib64",
        "/opt",
    ] {
        scan_cuda_dir(Path::new(pattern), &mut majors, 0);
    }
    majors.sort_unstable();
    majors
}

fn scan_cuda_dir(dir: &Path, majors: &mut Vec<u32>, depth: usize) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix("libcudart.so.") {
            let major: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(major) = major.parse::<u32>()
                && !majors.contains(&major)
            {
                majors.push(major);
            }
        } else if path.is_dir() && (name.starts_with("cuda") || name == "lib64") {
            scan_cuda_dir(&path, majors, depth + 1);
        }
    }
}

fn detect_glibc() -> Option<(u32, u32)> {
    if let Ok(output) = Command::new("getconf").arg("GNU_LIBC_VERSION").output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(version) = text.split_whitespace().nth(1)
            && let Some(parsed) = parse_version(version)
        {
            return Some(parsed);
        }
    }
    let output = Command::new("ldd").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?;
    let version = line.split_whitespace().last()?;
    parse_version(version)
}

fn has_library(name: &str) -> bool {
    if let Some(text) = ldconfig_output()
        && text.lines().any(|line| line.contains(name))
    {
        return true;
    }
    [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/lib/x86_64-linux-gnu",
    ]
    .iter()
    .any(|dir| Path::new(dir).join(name).exists())
}

fn ldconfig_output() -> Option<String> {
    for command in ["ldconfig", "/sbin/ldconfig", "/usr/sbin/ldconfig"] {
        if let Ok(output) = Command::new(command).arg("-p").output()
            && output.status.success()
        {
            return Some(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }
    None
}

fn parse_compute_cap(text: &str) -> Option<u32> {
    let (major, minor) = text.split_once('.')?;
    let major: u32 = major.trim().parse().ok()?;
    let minor: u32 = minor.trim().parse().ok()?;
    Some(major * 10 + minor)
}

fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compute_caps() {
        assert_eq!(parse_compute_cap("5.2"), Some(52));
        assert_eq!(parse_compute_cap("8.9"), Some(89));
        assert_eq!(parse_compute_cap("bad"), None);
    }

    #[test]
    fn checks_archs() {
        let entry = BackendEntry {
            id: "x".into(),
            label: "x".into(),
            kind: BackendKind::Cuda,
            version: "v".into(),
            url: "u".into(),
            sha256: "s".into(),
            size: 1,
            backend: None,
            cuda_major: Some(12),
            rocm_major: None,
            archs: vec![52, 61],
            requires: vec![],
            min_glibc: None,
            strip_prefix: None,
            notes: None,
        };
        assert!(entry.supports_archs(Some(52)));
        assert!(!entry.supports_archs(Some(89)));
        assert!(!entry.supports_archs(None));
    }

    #[test]
    fn bundled_manifest_is_valid() {
        let manifest = bundled_manifest();
        assert!(!manifest.backends.is_empty());
        assert!(manifest.cpu_entry().is_some());
        for entry in &manifest.backends {
            assert!(!entry.id.is_empty());
        }
    }
}
