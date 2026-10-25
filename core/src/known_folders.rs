//! Win32 "known folder" paths (`SHGetKnownFolderPath`), resolved lazily and
//! cached — one system call per [`KnownFolder`] actually asked for, not one
//! per folder this module knows how to resolve.
//!
//! The motivating case is `y-pop`'s `%win%`/`%mydocs%`/… builtin table: most
//! configuration files use a handful of these tokens, not all of them, and
//! resolving every one up front is Win32 traffic (and `CoTaskMemFree`
//! bookkeeping) spent on folders nothing ever reads.

use std::collections::HashMap;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_LocalAppData,
    FOLDERID_Music, FOLDERID_Profile, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86,
    FOLDERID_Programs, FOLDERID_RoamingAppData, FOLDERID_System, FOLDERID_SystemX86,
    FOLDERID_Videos, FOLDERID_Windows, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::GUID;

/// A `FOLDERID_*` Win32 knows about by name — a well-known Windows or user
/// directory, not an arbitrary path.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KnownFolder {
    Windows,
    System,
    SystemX86,
    ProgramFiles,
    ProgramFilesX86,
    Documents,
    Music,
    Videos,
    Desktop,
    LocalAppData,
    RoamingAppData,
    Profile,
    /// The current user's Start menu `Programs` folder.
    StartMenuPrograms,
    /// The Start menu `Programs` folder every user shares.
    CommonStartMenuPrograms,
}

impl KnownFolder {
    fn guid(self) -> &'static GUID {
        match self {
            Self::Windows => &FOLDERID_Windows,
            Self::System => &FOLDERID_System,
            Self::SystemX86 => &FOLDERID_SystemX86,
            Self::ProgramFiles => &FOLDERID_ProgramFiles,
            Self::ProgramFilesX86 => &FOLDERID_ProgramFilesX86,
            Self::Documents => &FOLDERID_Documents,
            Self::Music => &FOLDERID_Music,
            Self::Videos => &FOLDERID_Videos,
            Self::Desktop => &FOLDERID_Desktop,
            Self::LocalAppData => &FOLDERID_LocalAppData,
            Self::RoamingAppData => &FOLDERID_RoamingAppData,
            Self::Profile => &FOLDERID_Profile,
            Self::StartMenuPrograms => &FOLDERID_Programs,
            Self::CommonStartMenuPrograms => &FOLDERID_CommonPrograms,
        }
    }
}

/// Resolves [`KnownFolder`] paths on first request and remembers the
/// result — including a failure (`None`), so a folder Windows could not
/// resolve is not retried on every subsequent request for it either.
///
/// Cheap to create and drop per use: nothing here outlives the moment
/// nothing needs it any more, so a caller resolving a batch of tokens once
/// (parsing a config file, say) can build one, use it, and let it go —
/// there is no reason to keep it, or its cache, around any longer than that.
#[derive(Default)]
pub struct KnownFolders {
    cache: HashMap<KnownFolder, Option<Box<str>>>,
}

impl KnownFolders {
    pub fn new() -> Self {
        Self::default()
    }

    /// `folder`'s path, or `None` if Windows could not resolve it.
    pub fn get(&mut self, folder: KnownFolder) -> Option<&str> {
        self.cache
            .entry(folder)
            .or_insert_with(|| resolve(folder.guid()))
            .as_deref()
    }
}

fn resolve(id: &GUID) -> Option<Box<str>> {
    let path = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    let text = unsafe { path.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(path.0.cast())) };
    text.map(String::into_boxed_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_after_the_first_resolution() {
        let mut folders = KnownFolders::new();
        let first = folders.get(KnownFolder::Windows).map(str::to_owned);
        let second = folders.get(KnownFolder::Windows).map(str::to_owned);
        assert_eq!(first, second);
        assert_eq!(folders.cache.len(), 1); // one entry, not one per call
    }
}
