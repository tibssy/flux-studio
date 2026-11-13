//! Single source of truth for the app's user-visible identity.
//!
//! Renaming the app means editing this file (plus `packaging/`), not hunting
//! for strings across the UI, the history database and the filesystem paths.

/// Human-readable name shown in the window title, About page and desktop entry.
pub const APP_NAME: &str = "Flux Studio";

/// Reverse-DNS application id used by GApplication and the desktop entry.
pub const APP_ID: &str = "io.github.tibssy.FluxStudio";

/// Directory name under `$XDG_DATA_HOME`, `$XDG_CONFIG_HOME` and
/// `$XDG_CACHE_HOME`. Changing this relocates existing user data.
pub const APP_DIR: &str = "flux-studio";

/// Name of the installed executable.
pub const APP_BINARY: &str = "flux-studio";

/// Default folder (under the user's Pictures directory) for generated PNGs.
pub const DEFAULT_OUTPUT_FOLDER: &str = "Flux Studio";
