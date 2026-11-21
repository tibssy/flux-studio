//! Single source of truth for the app's user-visible identity.
//!
//! Renaming the app means editing this file (plus `packaging/`), not hunting
//! for strings across the UI, the history database and the filesystem paths.

/// Human-readable name shown in the window title, About page and desktop entry.
pub const APP_NAME: &str = "Imagiflume";

/// Reverse-DNS application id used by GApplication and the desktop entry.
pub const APP_ID: &str = "io.github.tibssy.Imagiflume";

/// Directory name under `$XDG_DATA_HOME`, `$XDG_CONFIG_HOME` and
/// `$XDG_CACHE_HOME`. Changing this relocates existing user data.
pub const APP_DIR: &str = "imagiflume";

/// Name of the installed executable.
pub const APP_BINARY: &str = "imagiflume";

/// Default folder (under the user's Pictures directory) for generated PNGs.
pub const DEFAULT_OUTPUT_FOLDER: &str = "Imagiflume";
