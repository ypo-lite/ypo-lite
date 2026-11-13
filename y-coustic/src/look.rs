//! What a row or the bar says about an application: its name and its icon.
//!
//! Resolved from the executable (the file description for the name, the file's
//! own icon) and kept by the caller between two readings of the same
//! application, so a wheel turning or a panel refreshing does not read the
//! version resource and extract the icon again every time.

use std::path::Path;

use y_core::file_info::file_description;
use y_core::icon::IconCache;

/// The application a reading is about: what the HUD names and draws the icon
/// of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRef {
    /// The executable, for the icon and the name; `None` for system sounds
    /// and for a session of no single process.
    pub exe: Option<String>,
    /// What to call it when the executable gives no name.
    pub name: String,
}

/// What the HUD resolved about an application: kept between two readings of
/// the same one, so a wheel turning does not read the executable's icon and
/// version resource at every notch.
pub struct Look {
    pub app: AppRef,
    /// The icon size it was rendered at, in pixels.
    pub size: i32,
    pub name: String,
    pub pixels: Option<Vec<u32>>,
}

impl Look {
    pub fn resolve(app: &AppRef, size: i32) -> Self {
        let path = app.exe.as_deref();
        let name = path
            .and_then(|path| file_description(Path::new(path)))
            .or_else(|| {
                path.and_then(|path| Path::new(path).file_stem())
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| app.name.clone());
        let pixels = path.and_then(|path| {
            IconCache::with_size(size)
                .for_file(path, None)
                .and_then(|icon| icon.to_pixels(size))
        });
        Self {
            app: app.clone(),
            size,
            name,
            pixels,
        }
    }
}
