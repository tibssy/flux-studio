use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::paths;

/// Default manifest location. The bundled copy is used when this cannot be
/// fetched or when the app is offline.
pub const DEFAULT_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/tibssy/flux-studio/main/backends/manifest.json";

/// Default location of the llama.cpp backend manifest.
pub const DEFAULT_LLAMA_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/tibssy/flux-studio/main/backends/llama-manifest.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    Cpu,
    Vulkan,
    Rocm,
    Cuda,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArchiveFormat {
    #[default]
    Zip,
    TarGz,
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
    /// Executable to install and run. Defaults to `sd-server`.
    #[serde(default)]
    pub binary: Option<String>,
    /// Archive format. Inferred from the URL when omitted.
    #[serde(default)]
    pub archive: Option<ArchiveFormat>,
}

impl BackendEntry {
    pub fn install_dir(&self, root: &Path) -> PathBuf {
        root.join(&self.id)
    }

    pub fn binary_name(&self) -> &str {
        self.binary.as_deref().unwrap_or("sd-server")
    }

    pub fn binary_path(&self, root: &Path) -> PathBuf {
        self.install_dir(root).join(self.binary_name())
    }

    pub fn archive_format(&self) -> ArchiveFormat {
        self.archive.unwrap_or_else(|| {
            if self.url.ends_with(".tar.gz") {
                ArchiveFormat::TarGz
            } else {
                ArchiveFormat::Zip
            }
        })
    }

    pub fn marker_path(&self, root: &Path) -> PathBuf {
        self.install_dir(root).join("installed.json")
    }

    pub fn is_installed(&self, root: &Path) -> bool {
        self.binary_path(root).is_file()
    }

    pub fn installed_info(&self, root: &Path) -> Option<InstalledBackend> {
        read_installed(&self.marker_path(root))
    }

    pub fn installed_version(&self, root: &Path) -> Option<String> {
        self.installed_info(root).map(|marker| marker.version)
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
    /// Archive size, used for the installed/available display.
    #[serde(default)]
    pub size: u64,
}

pub fn read_installed(path: &Path) -> Option<InstalledBackend> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
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

    /// Parse a remote manifest entry-by-entry so one corrupt backend does not
    /// discard the valid ones.
    pub fn parse_lenient(text: &str) -> anyhow::Result<(Self, Vec<String>)> {
        let value: serde_json::Value = serde_json::from_str(text)?;
        let schema_version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        anyhow::ensure!(
            schema_version == 1,
            "unsupported backend manifest schema {schema_version}"
        );
        let generated_at = value
            .get("generated_at")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let Some(items) = value.get("backends").and_then(serde_json::Value::as_array) else {
            anyhow::bail!("manifest has no backends array");
        };

        let mut backends = Vec::new();
        let mut errors = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let mut entry: BackendEntry = match serde_json::from_value(item.clone()) {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(format!("entry {index}: {error}"));
                    continue;
                }
            };
            entry.url = entry.url.trim().to_string();
            entry.sha256 = entry.sha256.trim().to_ascii_lowercase();
            if let Err(error) = validate_entry(&entry) {
                errors.push(format!("entry {index} ({}): {error}", entry.id));
                continue;
            }
            backends.push(entry);
        }

        Ok((
            Self {
                schema_version: 1,
                generated_at,
                backends,
            },
            errors,
        ))
    }

    /// Overlay valid remote entries on the pinned list. Entries that were
    /// skipped during parsing keep their pinned version.
    pub fn merge(base: &Self, overlay: &Self) -> Self {
        let mut backends = base.backends.clone();
        for entry in &overlay.backends {
            if let Some(existing) = backends.iter_mut().find(|existing| existing.id == entry.id) {
                *existing = entry.clone();
            } else {
                backends.push(entry.clone());
            }
        }
        Self {
            schema_version: 1,
            generated_at: overlay.generated_at.clone(),
            backends,
        }
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

fn validate_entry(entry: &BackendEntry) -> anyhow::Result<()> {
    anyhow::ensure!(!entry.id.trim().is_empty(), "empty id");
    anyhow::ensure!(!entry.label.trim().is_empty(), "empty label");
    anyhow::ensure!(!entry.version.trim().is_empty(), "empty version");

    let has_url = !entry.url.is_empty();
    let has_checksum = !entry.sha256.is_empty();
    let has_size = entry.size > 0;
    if has_url || has_checksum || has_size {
        anyhow::ensure!(
            entry.url.starts_with("https://") || entry.url.starts_with("http://"),
            "invalid download URL"
        );
        anyhow::ensure!(
            entry.sha256.len() == 64 && entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid SHA-256"
        );
        anyhow::ensure!(entry.size > 0, "missing archive size");
    }
    Ok(())
}

pub fn bundled_manifest() -> BackendManifest {
    BackendManifest::parse(include_str!("../../../backends/manifest.json"))
        .expect("bundled backend manifest is valid")
}

pub fn bundled_llama_manifest() -> BackendManifest {
    BackendManifest::parse(include_str!("../../../backends/llama-manifest.json"))
        .expect("bundled llama.cpp manifest is valid")
}

#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub name: String,
    pub compute_cap: Option<u32>,
}

/// What NVML reports for the first NVIDIA GPU.
#[derive(Debug, Clone, Default)]
struct NvmlInfo {
    gpu: GpuInfo,
    /// CUDA version supported by the installed driver, encoded as NVML
    /// reports it (for example 12080 for 12.8).
    cuda_driver: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct HardwareProfile {
    pub nvidia: Option<GpuInfo>,
    pub amd: bool,
    pub vulkan: bool,
    /// CUDA runtime majors found as `libcudart.so.<major>`.
    pub cuda_majors: Vec<u32>,
    /// CUDA version supported by the NVIDIA driver. The runtime may still be
    /// absent, so this is not the same as `cuda_majors`.
    pub cuda_driver: Option<u32>,
    pub glibc: Option<(u32, u32)>,
}

impl HardwareProfile {
    pub fn detect() -> Self {
        let nvml = detect_nvml();
        Self {
            nvidia: detect_nvidia_smi()
                .or_else(|| nvml.as_ref().map(|info| info.gpu.clone()))
                .or_else(detect_nvidia_sysfs),
            amd: detect_amd(),
            vulkan: has_library("libvulkan.so.1"),
            cuda_majors: detect_cuda_majors(),
            cuda_driver: nvml.as_ref().and_then(|info| info.cuda_driver),
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

        let compatibility = match entry.kind {
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
                    Some(format!("CUDA {major} runtime not found (found: {found})"))
                } else if let Some(cap) = gpu.compute_cap
                    && !entry.supports_archs(Some(cap))
                {
                    Some(format!(
                        "This build does not include sm_{cap} (architectures: {})",
                        entry
                            .archs
                            .iter()
                            .map(|arch| format!("sm_{arch}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                } else {
                    None
                }
            }
        };
        if compatibility.is_some() {
            return compatibility;
        }

        // The manifest lists the sonames a build needs; trust it over the
        // broad GPU check above.
        if let Some(missing) = entry
            .requires
            .iter()
            .find(|library| !self.requirement_met(library))
        {
            return Some(format!("{missing} not found"));
        }
        None
    }

    fn requirement_met(&self, library: &str) -> bool {
        if let Some(major) = library
            .strip_prefix("libcudart.so.")
            .and_then(|major| major.parse::<u32>().ok())
        {
            return self.cuda_majors.contains(&major);
        }
        has_library(library)
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(gpu) = &self.nvidia {
            let name = gpu.name.strip_prefix("NVIDIA ").unwrap_or(&gpu.name);
            match gpu.compute_cap {
                Some(cap) => parts.push(format!("NVIDIA {name} (sm_{cap})")),
                None => parts.push(format!("NVIDIA {name}")),
            }
        }
        if self.amd {
            parts.push("AMD GPU".into());
        }
        if self.vulkan {
            parts.push("Vulkan".into());
        }
        if let Some(version) = self.cuda_driver {
            let label = cuda_driver_label(version);
            if self.cuda_majors.is_empty() {
                parts.push(format!("{label} (runtime missing)"));
            } else {
                parts.push(label);
            }
        }
        if !self.cuda_majors.is_empty() {
            parts.push(format!(
                "CUDA {} runtime",
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

/// Places a system-installed `llama-cli` is commonly found.
pub fn system_llama_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("llama-cli"));
        candidates.push(dir.join("llama.cpp").join("llama-cli"));
        candidates.push(dir.join("../lib/flux-studio/llama-cli"));
        candidates.push(dir.join("../libexec/flux-studio/llama-cli"));
    }
    candidates.push(PathBuf::from("/usr/lib/flux-studio/llama-cli"));
    candidates.push(PathBuf::from("/usr/lib/llama.cpp/llama-cli"));
    candidates.push(PathBuf::from("/usr/lib64/llama.cpp/llama-cli"));
    candidates.push(PathBuf::from("/usr/libexec/llama.cpp/llama-cli"));
    candidates.push(PathBuf::from("/usr/local/lib/flux-studio/llama-cli"));
    candidates.push(PathBuf::from("/usr/local/bin/llama-cli"));
    candidates.push(PathBuf::from("/usr/bin/llama-cli"));
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/llama-cli"));
        candidates.push(home.join(".local/lib/flux-studio/llama-cli"));
    }
    candidates
}

pub fn find_system_llama_cli() -> Option<PathBuf> {
    system_llama_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| find_in_path("llama-cli"))
}

/// Check that a binary can at least load its shared libraries.
pub fn binary_works(path: &Path) -> bool {
    path.is_file()
        && std::process::Command::new(path)
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
}

pub fn server_works(path: &Path) -> bool {
    binary_works(path)
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

pub fn llama_backends_dir() -> PathBuf {
    paths::backends_dir().join("llama")
}

#[cfg(unix)]
fn detect_nvml() -> Option<NvmlInfo> {
    nvml::detect()
}

#[cfg(not(unix))]
fn detect_nvml() -> Option<NvmlInfo> {
    None
}

fn detect_nvidia_smi() -> Option<GpuInfo> {
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

/// Read the model name the NVIDIA driver exposes through sysfs/procfs.
///
/// The sandbox may have neither `nvidia-smi` nor a loadable NVML library, but
/// `/sys/class/drm` and `/proc/driver/nvidia` are visible. The compute
/// capability is unknown in that case, which is reported as-is instead of as
/// `sm_0`.
fn detect_nvidia_sysfs() -> Option<GpuInfo> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(rest) = name.strip_prefix("card") else {
            continue;
        };
        if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let device = entry.path().join("device");
        let Ok(vendor) = std::fs::read_to_string(device.join("vendor")) else {
            continue;
        };
        if !vendor.trim().eq_ignore_ascii_case("0x10de") {
            continue;
        }
        let model = std::fs::canonicalize(&device)
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .and_then(|address| {
                std::fs::read_to_string(format!("/proc/driver/nvidia/gpus/{address}/information"))
                    .ok()
            })
            .and_then(|information| nvidia_model(&information));
        return Some(GpuInfo {
            name: model.unwrap_or_else(|| "GPU".into()),
            compute_cap: None,
        });
    }
    None
}

fn nvidia_model(information: &str) -> Option<String> {
    information.lines().find_map(|line| {
        line.strip_prefix("Model:")
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string)
    })
}

/// Minimal NVML binding, loaded at runtime.
///
/// The Flatpak does not expose `nvidia-smi`, and normal library paths do not
/// include the driver extension, so the exact path is resolved first. Only a
/// handful of entry points are needed, which avoids a build-time dependency
/// on the driver headers.
#[cfg(unix)]
mod nvml {
    use super::{GpuInfo, NvmlInfo};
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_uint, c_void};
    use std::path::{Path, PathBuf};

    type Device = *mut c_void;
    type Init = unsafe extern "C" fn() -> c_int;
    type Shutdown = unsafe extern "C" fn() -> c_int;
    type GetCount = unsafe extern "C" fn(*mut c_uint) -> c_int;
    type GetHandle = unsafe extern "C" fn(c_uint, *mut Device) -> c_int;
    type GetName = unsafe extern "C" fn(Device, *mut c_char, c_uint) -> c_int;
    type GetCapability = unsafe extern "C" fn(Device, *mut c_int, *mut c_int) -> c_int;
    type GetCudaDriver = unsafe extern "C" fn(*mut c_int) -> c_int;

    const SUCCESS: c_int = 0;

    pub(super) fn detect() -> Option<NvmlInfo> {
        candidates().iter().find_map(|path| detect_from(path))
    }

    fn detect_from(path: &Path) -> Option<NvmlInfo> {
        let path = CString::new(path.to_string_lossy().as_bytes()).ok()?;
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW) };
        if handle.is_null() {
            return None;
        }
        let result = query(handle);
        unsafe { libc::dlclose(handle) };
        result
    }

    fn query(handle: *mut c_void) -> Option<NvmlInfo> {
        let init: Init = load(handle, b"nvmlInit_v2\0")?;
        let shutdown: Shutdown = load(handle, b"nvmlShutdown\0")?;
        let get_count: GetCount = load(handle, b"nvmlDeviceGetCount_v2\0")?;
        let get_handle: GetHandle = load(handle, b"nvmlDeviceGetHandleByIndex_v2\0")?;
        let get_name: GetName = load(handle, b"nvmlDeviceGetName\0")?;
        let get_capability: GetCapability = load(handle, b"nvmlDeviceGetCudaComputeCapability\0")?;
        let get_cuda_driver: Option<GetCudaDriver> =
            load(handle, b"nvmlSystemGetCudaDriverVersion_v2\0")
                .or_else(|| load(handle, b"nvmlSystemGetCudaDriverVersion\0"));

        if unsafe { init() } != SUCCESS {
            return None;
        }
        let result = (|| {
            let mut count = 0;
            if unsafe { get_count(&mut count) } != SUCCESS || count == 0 {
                return None;
            }
            let mut device: Device = std::ptr::null_mut();
            if unsafe { get_handle(0, &mut device) } != SUCCESS || device.is_null() {
                return None;
            }

            let mut name = [0 as c_char; 128];
            let name = if unsafe { get_name(device, name.as_mut_ptr(), name.len() as c_uint) }
                == SUCCESS
            {
                unsafe { CStr::from_ptr(name.as_ptr()) }
                    .to_string_lossy()
                    .into_owned()
            } else {
                "GPU".to_string()
            };

            let mut major = 0;
            let mut minor = 0;
            let compute_cap =
                if unsafe { get_capability(device, &mut major, &mut minor) } == SUCCESS {
                    Some((major.max(0) as u32) * 10 + minor.max(0) as u32)
                } else {
                    None
                };

            let cuda_driver = get_cuda_driver.and_then(|get_version| {
                let mut version = 0;
                (unsafe { get_version(&mut version) } == SUCCESS && version > 0)
                    .then_some(version as u32)
            });

            Some(NvmlInfo {
                gpu: GpuInfo { name, compute_cap },
                cuda_driver,
            })
        })();
        unsafe { shutdown() };
        result
    }

    fn load<T: Copy>(handle: *mut c_void, name: &[u8]) -> Option<T> {
        let symbol = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
        (!symbol.is_null()).then(|| unsafe { std::mem::transmute_copy(&symbol) })
    }

    fn candidates() -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from("libnvidia-ml.so.1")];
        for directory in [
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
            "/usr/lib64",
            "/usr/lib",
            "/usr/local/lib",
        ] {
            paths.push(PathBuf::from(directory).join("libnvidia-ml.so.1"));
        }
        // The Flatpak driver extension keeps the libraries out of the normal
        // search path.
        for root in [
            "/usr/lib/x86_64-linux-gnu/GL",
            "/usr/lib/aarch64-linux-gnu/GL",
            "/usr/lib64/GL",
            "/usr/lib/GL",
        ] {
            let Ok(entries) = std::fs::read_dir(root) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("nvidia-") {
                    paths.push(entry.path().join("lib").join("libnvidia-ml.so.1"));
                }
            }
        }
        paths
    }
}

fn detect_amd() -> bool {
    detect_amd_in(Path::new("/sys/class/drm"))
}

fn detect_amd_in(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(rest) = name.strip_prefix("card") else {
            continue;
        };
        if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
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
    if [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/lib/x86_64-linux-gnu",
    ]
    .iter()
    .any(|dir| Path::new(dir).join(name).exists())
    {
        return true;
    }
    // CUDA and ROCm toolkits are commonly installed outside the loader path.
    if let Ok(entries) = std::fs::read_dir("/opt") {
        for entry in entries.flatten() {
            let label = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if (label.starts_with("cuda") || label.starts_with("rocm"))
                && find_library(&entry.path(), name, 0)
            {
                return true;
            }
        }
    }
    false
}

fn find_library(dir: &Path, name: &str, depth: usize) -> bool {
    if depth > 3 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|file| file.to_string_lossy() == name)
        {
            return true;
        }
        if path.is_dir() && find_library(&path, name, depth + 1) {
            return true;
        }
    }
    false
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

/// Format NVML's integer CUDA version (12080) as `CUDA 12.8 driver`.
fn cuda_driver_label(version: u32) -> String {
    let major = version / 1000;
    let minor = (version % 1000) / 10;
    format!("CUDA {major}.{minor} driver")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry(kind: BackendKind) -> BackendEntry {
        BackendEntry {
            id: "test".into(),
            label: "Test".into(),
            kind,
            version: "v".into(),
            url: "u".into(),
            sha256: "s".into(),
            size: 1,
            backend: None,
            cuda_major: None,
            rocm_major: None,
            archs: vec![],
            requires: vec![],
            min_glibc: None,
            strip_prefix: None,
            notes: None,
            binary: None,
            archive: None,
        }
    }

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
            binary: None,
            archive: None,
        };
        assert!(entry.supports_archs(Some(52)));
        assert!(!entry.supports_archs(Some(89)));
        assert!(!entry.supports_archs(None));
    }

    #[test]
    fn parses_the_nvidia_model_from_procfs() {
        let information = "\
Model: \t\t Quadro M4000
IRQ:   \t\t 163
GPU UUID: \t GPU-250c6a35
";
        assert_eq!(nvidia_model(information), Some("Quadro M4000".into()));
        assert_eq!(nvidia_model("IRQ: 1"), None);
    }

    #[test]
    fn summary_only_includes_a_known_compute_cap() {
        let known = HardwareProfile {
            nvidia: Some(GpuInfo {
                name: "Quadro M4000".into(),
                compute_cap: Some(52),
            }),
            ..Default::default()
        };
        assert_eq!(known.summary(), "NVIDIA Quadro M4000 (sm_52) · CPU");

        let unknown = HardwareProfile {
            nvidia: Some(GpuInfo {
                name: "Quadro M4000".into(),
                compute_cap: None,
            }),
            ..Default::default()
        };
        assert_eq!(unknown.summary(), "NVIDIA Quadro M4000 · CPU");
    }

    #[test]
    fn summary_separates_driver_and_runtime_cuda_versions() {
        let hardware = HardwareProfile {
            cuda_driver: Some(13000),
            cuda_majors: vec![12],
            ..Default::default()
        };
        assert_eq!(
            hardware.summary(),
            "CUDA 13.0 driver · CUDA 12 runtime · CPU"
        );
        assert_eq!(cuda_driver_label(12080), "CUDA 12.8 driver");

        let driver_only = HardwareProfile {
            cuda_driver: Some(13000),
            ..Default::default()
        };
        assert_eq!(
            driver_only.summary(),
            "CUDA 13.0 driver (runtime missing) · CPU"
        );
    }

    #[test]
    fn detect_amd_ignores_connectors_and_other_vendors() {
        let dir = std::env::temp_dir().join(format!("flux-amd-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let connector = dir.join("card0-DP-1/device");
        std::fs::create_dir_all(&connector).unwrap();
        std::fs::write(connector.join("vendor"), "0x1002\n").unwrap();
        let nvidia = dir.join("card0/device");
        std::fs::create_dir_all(&nvidia).unwrap();
        std::fs::write(nvidia.join("vendor"), "0x10de\n").unwrap();
        assert!(!detect_amd_in(&dir));

        let amd = dir.join("card1/device");
        std::fs::create_dir_all(&amd).unwrap();
        std::fs::write(amd.join("vendor"), "0x1002\n").unwrap();
        assert!(detect_amd_in(&dir));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finds_toolkit_libraries_outside_the_loader_path() {
        let root = std::env::temp_dir().join(format!("flux-lib-scan-{}", std::process::id()));
        let lib = root.join("cuda-13.0/lib64");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("libcudart.so.13"), b"x").unwrap();

        assert!(find_library(&root, "libcudart.so.13", 0));
        assert!(!find_library(&root, "libcudart.so.12", 0));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_required_library_blocks_a_backend() {
        let mut entry = test_entry(BackendKind::Cpu);
        entry.requires = vec!["libflux-missing-test.so.99".into()];
        assert_eq!(
            HardwareProfile::default().compatibility(&entry),
            Some("libflux-missing-test.so.99 not found".into())
        );
    }

    #[test]
    fn cudart_requirement_uses_the_detected_runtime() {
        let mut entry = test_entry(BackendKind::Cpu);
        entry.requires = vec!["libcudart.so.12".into()];
        let present = HardwareProfile {
            cuda_majors: vec![12],
            ..Default::default()
        };
        assert_eq!(present.compatibility(&entry), None);

        let absent = HardwareProfile {
            cuda_majors: vec![11],
            ..Default::default()
        };
        assert_eq!(
            absent.compatibility(&entry),
            Some("libcudart.so.12 not found".into())
        );
    }

    #[test]
    fn unknown_compute_cap_does_not_reject_cuda_backends() {
        let entry = BackendEntry {
            id: "cuda12".into(),
            label: "CUDA 12".into(),
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
            binary: None,
            archive: None,
        };
        let hardware = HardwareProfile {
            nvidia: Some(GpuInfo {
                name: "Quadro M4000".into(),
                compute_cap: None,
            }),
            cuda_majors: vec![12],
            ..Default::default()
        };
        assert_eq!(hardware.compatibility(&entry), None);
    }

    #[test]
    fn lenient_parse_skips_only_bad_entries() {
        let manifest = r#"{
            "schema_version": 1,
            "generated_at": "test",
            "backends": [
                {"id":"cpu","label":"CPU","kind":"cpu","version":"v1","url":"https://example.com/cpu.zip","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":10},
                {"id":"bad","label":"Bad","kind":"vulkan","version":"v1","url":"not-a-url","sha256":"zz","size":10},
                {"id":"cuda","label":"CUDA","kind":"cuda","version":"unpublished","url":"","sha256":"","size":0}
            ]
        }"#;
        let (manifest, errors) = BackendManifest::parse_lenient(manifest).unwrap();
        assert_eq!(manifest.backends.len(), 2);
        assert_eq!(errors.len(), 1);
        assert!(manifest.entry("cpu").is_some());
        assert!(manifest.entry("bad").is_none());
        assert!(manifest.entry("cuda").is_some());
    }

    #[test]
    fn merge_keeps_pinned_entries_and_adds_new_ones() {
        let base = BackendManifest {
            schema_version: 1,
            generated_at: "base".into(),
            backends: vec![BackendEntry {
                id: "cpu".into(),
                label: "CPU".into(),
                kind: BackendKind::Cpu,
                version: "old".into(),
                url: "https://example.com/old.zip".into(),
                sha256: "a".repeat(64),
                size: 1,
                backend: Some("cpu".into()),
                cuda_major: None,
                rocm_major: None,
                archs: vec![],
                requires: vec![],
                min_glibc: None,
                strip_prefix: None,
                notes: None,
                binary: None,
                archive: None,
            }],
        };
        let overlay = BackendManifest {
            schema_version: 1,
            generated_at: "remote".into(),
            backends: vec![
                BackendEntry {
                    version: "new".into(),
                    url: "https://example.com/new.zip".into(),
                    sha256: "b".repeat(64),
                    ..base.backends[0].clone()
                },
                BackendEntry {
                    id: "vulkan".into(),
                    label: "Vulkan".into(),
                    kind: BackendKind::Vulkan,
                    version: "new".into(),
                    url: "https://example.com/vk.zip".into(),
                    sha256: "c".repeat(64),
                    size: 1,
                    backend: Some("vulkan0".into()),
                    cuda_major: None,
                    rocm_major: None,
                    archs: vec![],
                    requires: vec![],
                    min_glibc: None,
                    strip_prefix: None,
                    notes: None,
                    binary: None,
                    archive: None,
                },
            ],
        };
        let merged = BackendManifest::merge(&base, &overlay);
        assert_eq!(merged.backends.len(), 2);
        assert_eq!(merged.entry("cpu").unwrap().version, "new");
        assert!(merged.entry("vulkan").is_some());
    }

    #[test]
    fn bundled_manifest_is_valid() {
        let manifest = bundled_manifest();
        assert!(!manifest.backends.is_empty());
        assert!(manifest.cpu_entry().is_some());
        for entry in &manifest.backends {
            assert!(!entry.id.is_empty());
            assert_eq!(entry.binary_name(), "sd-server");
            assert_eq!(entry.archive_format(), ArchiveFormat::Zip);
        }
    }

    #[test]
    fn bundled_llama_manifest_is_valid() {
        let manifest = bundled_llama_manifest();
        assert!(!manifest.backends.is_empty());
        assert!(manifest.cpu_entry().is_some());
        for entry in &manifest.backends {
            assert!(!entry.id.is_empty());
            assert_eq!(entry.binary_name(), "llama-cli");
            assert_eq!(entry.archive_format(), ArchiveFormat::TarGz);
        }
    }

    #[test]
    fn bundled_binaries_are_discovered_next_to_the_app() {
        // The Flatpak installs both backends in /app/lib/flux-studio.
        assert!(
            system_server_candidates()
                .iter()
                .any(|path| path.ends_with("lib/flux-studio/sd-server"))
        );
        assert!(
            system_llama_candidates()
                .iter()
                .any(|path| path.ends_with("lib/flux-studio/llama-cli"))
        );
    }

    #[test]
    fn published_cuda_entries_are_available() {
        let manifest = bundled_manifest();
        let cuda12 = manifest.entry("cuda12").expect("cuda12 entry");
        assert!(cuda12.is_available());
        assert_eq!(cuda12.version, "master-1094-3bad892");
        assert_eq!(cuda12.archs, vec![52, 61, 70]);
        let cuda13 = manifest.entry("cuda13").expect("cuda13 entry");
        assert!(cuda13.is_available());
        assert_eq!(cuda13.version, "master-1094-3bad892");
    }

    #[test]
    fn archive_format_is_inferred_from_the_url() {
        let mut entry = bundled_manifest().backends[0].clone();
        entry.archive = None;
        entry.url = "https://example.com/backend.tar.gz".into();
        assert_eq!(entry.archive_format(), ArchiveFormat::TarGz);
        entry.url = "https://example.com/backend.zip".into();
        assert_eq!(entry.archive_format(), ArchiveFormat::Zip);
    }
}
