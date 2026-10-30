//! What Windows itself remembers having opened: the source of the Start menu's
//! "recent" and "most used" lists.
//!
//! # Files
//!
//! The current user's `Recent` folder holds one shortcut per recently opened
//! file or folder, named after it, its time being the last opening. Listing
//! the folder is all [`files`] does: no shortcut is opened, so a slow or
//! vanished target costs nothing. [`add_file`] is the documented way in
//! (`SHAddToRecentDocs`).
//!
//! # Programs
//!
//! Programs are in `UserAssist`, under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist`.
//! **It is undocumented**, though unchanged since Windows 7: one key of
//! programs and one of shortcuts, each value named after the thing in ROT13
//! and holding a 72-byte record. Only two things in it are relied on:
//!
//! * the name, which is a path, a path under a known folder
//!   (`{1AC14E77-…}\notepad.exe`) or an AppUserModelID, the very ids of
//!   `shell:AppsFolder`;
//! * the last-run time at offset 60, a `FILETIME`.
//!
//! The run count at offset 4 is often 0 for things that did run (11 of 187
//! records on the machine this was written on), so a record is judged by its
//! time. Nothing here writes a record: the way in is
//! `SEE_MASK_FLAG_LOG_USAGE`, on the launch itself.
//!
//! # Privacy
//!
//! The user can turn both lists off (Settings, Personalization, Start).
//! Windows then records nothing and these functions find nothing; there is
//! nothing to work around. [`tracks_files`] says why a list is empty.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_VALUE_TYPE, RRF_RT_REG_DWORD,
    RegCloseKey, RegDeleteValueW, RegEnumValueW, RegGetValueW, RegOpenKeyExW,
};
use windows::Win32::UI::Shell::{FOLDERID_Recent, KF_FLAG_DEFAULT, SHARD_PATHW, SHAddToRecentDocs};
use windows::core::{GUID, HSTRING, PCWSTR, PWSTR, w};

use crate::known_folders;

const PROGRAMS: PCWSTR = w!(
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\UserAssist\\{CEBFF5CD-ACE2-4F4F-9178-9926F41749EA}\\Count"
);
const SHORTCUTS: PCWSTR = w!(
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\UserAssist\\{F4E57C4B-2036-45F0-A9AB-443BCFE33D9F}\\Count"
);

/// How long a `UserAssist` record is.
const RECORD: usize = 72;
const RECORD_COUNT: usize = 4;
const RECORD_TIME: usize = 60;
/// The longest a registry value's name can be, in characters.
const MAX_NAME: usize = 16_384;
/// Seconds from 1601 (a `FILETIME`'s origin) to 1970.
const EPOCH_GAP: u64 = 11_644_473_600;

/// A program Windows saw launched.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Launch {
    /// What it is called in `UserAssist`, decoded: a path, a path under a
    /// known folder, or an id of `shell:AppsFolder`.
    pub id: String,
    /// The value it is stored under, for [`forget_launch`].
    value: String,
    /// In the shortcuts' key rather than the programs'.
    pub shortcut: bool,
    /// Often 0: see the module doc.
    pub count: u32,
    /// Seconds since 1970.
    pub last: u64,
}

impl Launch {
    /// A launch of `id` that is not read from the registry, for tests.
    pub fn new(id: &str, count: u32, last: u64) -> Self {
        Self {
            id: id.to_owned(),
            value: rot13(id),
            shortcut: false,
            count,
            last,
        }
    }

    /// The file it stands for, known folders expanded. `None` for an
    /// AppUserModelID.
    pub fn path(&self) -> Option<String> {
        expanded_path(&self.id)
    }
}

/// A shortcut of the `Recent` folder.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecentFile {
    /// The shortcut itself. Opening it opens the file, and reading it
    /// ([`crate::file_info::read_shell_link`]) says where the file is.
    pub link: PathBuf,
    /// What was opened: `report.docx`.
    pub name: String,
    /// Seconds since 1970.
    pub last: u64,
}

/// Every program with a last-run time, newest first.
pub fn launches() -> Vec<Launch> {
    let mut found = read_key(PROGRAMS, false);
    found.extend(read_key(SHORTCUTS, true));
    found.sort_by_key(|launch| std::cmp::Reverse(launch.last));
    found
}

/// The shortcuts of the `Recent` folder, newest first. Sub-folders (the jump
/// lists' stores) and other files (`desktop.ini`) are left out.
pub fn files() -> Vec<RecentFile> {
    let Some(folder) = known_folders::resolve_id(&FOLDERID_Recent, KF_FLAG_DEFAULT) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&*folder) else {
        return Vec::new();
    };
    let mut found: Vec<RecentFile> = entries
        .flatten()
        .filter_map(|entry| {
            let link = entry.path();
            // No need to open the file: the listing already has its time.
            let metadata = entry.metadata().ok()?;
            let name = link_name(&link)?.to_owned();
            let last = metadata
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs();
            metadata
                .is_file()
                .then_some(RecentFile { link, name, last })
        })
        .collect();
    found.sort_by_key(|file| std::cmp::Reverse(file.last));
    found
}

/// Adds `path` to the files Windows remembers. Windows ignores it when the
/// user has turned that list off, and ignores executables always.
pub fn add_file(path: &Path) {
    let path = HSTRING::from(path.as_os_str());
    // SAFETY: `path` is a valid wide string, as `SHARD_PATHW` asks, and the
    // call does not keep it.
    unsafe { SHAddToRecentDocs(SHARD_PATHW.0 as u32, Some(path.as_ptr().cast())) };
}

/// Whether Windows keeps recent files at all (the "Show recently opened items"
/// switch). When it does not, [`files`] is empty and [`add_file`] does nothing.
pub fn tracks_files() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` is valid for the four bytes `size` announces.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"),
            w!("Start_TrackDocs"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    // No value at all is Windows's default: on.
    status != ERROR_SUCCESS || value != 0
}

/// Removes `launch` from what Windows remembers, as *Remove from list* does in
/// the Start menu. `false` if it could not.
pub fn forget_launch(launch: &Launch) -> bool {
    let key_path = if launch.shortcut { SHORTCUTS } else { PROGRAMS };
    let Some(key) = Key::open(key_path, KEY_SET_VALUE.0) else {
        return false;
    };
    // SAFETY: the key is open and the name a valid wide string.
    let status = unsafe { RegDeleteValueW(key.0, &HSTRING::from(launch.value.as_str())) };
    status == ERROR_SUCCESS
}

/// Removes a file from what Windows remembers. The file itself stays: this
/// deletes the shortcut to it.
pub fn forget_file(file: &RecentFile) -> bool {
    std::fs::remove_file(&file.link).is_ok()
}

fn read_key(path: PCWSTR, shortcut: bool) -> Vec<Launch> {
    let Some(key) = Key::open(path, KEY_QUERY_VALUE.0) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut name = vec![0u16; MAX_NAME];
    let mut data = [0u8; RECORD];
    let mut index = 0;
    loop {
        let mut name_length = name.len() as u32;
        let mut data_length = data.len() as u32;
        let mut kind = REG_VALUE_TYPE(0).0;
        // SAFETY: both buffers are ours and their lengths are passed with them.
        let status = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_length,
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut data_length),
            )
        };
        index += 1;
        if status == ERROR_NO_MORE_ITEMS {
            return found;
        }
        // Longer than a record (`ERROR_MORE_DATA`) is the key's own bookkeeping.
        if status != ERROR_SUCCESS || data_length as usize != RECORD {
            continue;
        }
        let value = String::from_utf16_lossy(&name[..name_length as usize]);
        let id = rot13(&value);
        if id.starts_with("UEME_") {
            continue;
        }
        if let Some((count, last)) = parse_record(&data) {
            found.push(Launch {
                id,
                value,
                shortcut,
                count,
                last,
            });
        }
    }
}

/// The run count and last-run time of a record, or `None` if it has no time:
/// Windows knows of it, but not that it ever ran.
fn parse_record(record: &[u8]) -> Option<(u32, u64)> {
    if record.len() != RECORD {
        return None;
    }
    let count = u32::from_le_bytes(record[RECORD_COUNT..RECORD_COUNT + 4].try_into().ok()?);
    let time = u64::from_le_bytes(record[RECORD_TIME..RECORD_TIME + 8].try_into().ok()?);
    let last = (time / 10_000_000).checked_sub(EPOCH_GAP)?;
    (last > 0).then_some((count, last))
}

/// Letters moved 13 places: both the encoding of the names and its decoding.
fn rot13(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            _ => c,
        })
        .collect()
}

/// `report.docx` for `…\Recent\report.docx.lnk`. `None` for anything else.
fn link_name(link: &Path) -> Option<&str> {
    let is_link = link
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));
    is_link.then(|| link.file_stem()?.to_str()).flatten()
}

/// `id` as a file: as it is when absolute, under its known folder when it is
/// written `{guid}\rest`. `None` for what is not a path.
fn expanded_path(id: &str) -> Option<String> {
    if let Some((folder, rest)) = known_folder_id(id) {
        let folder = GUID::try_from(folder).ok()?;
        let base = known_folders::resolve_id(&folder, KF_FLAG_DEFAULT)?;
        return Some(format!("{base}\\{rest}"));
    }
    let bytes = id.as_bytes();
    let absolute = bytes.len() > 2 && bytes[1] == b':' && bytes[2] == b'\\';
    (absolute || id.starts_with(r"\\")).then(|| id.to_owned())
}

/// The two halves of `{guid}\rest`, without the braces.
fn known_folder_id(id: &str) -> Option<(&str, &str)> {
    let (folder, rest) = id.split_once('\\')?;
    Some((folder.strip_prefix('{')?.strip_suffix('}')?, rest))
}

/// Closes the registry key on drop.
struct Key(HKEY);

impl Key {
    fn open(path: PCWSTR, access: u32) -> Option<Self> {
        let mut key = HKEY::default();
        // SAFETY: `path` is a valid wide string and `key` ours to fill.
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                path,
                None,
                windows::Win32::System::Registry::REG_SAM_FLAGS(access),
                &mut key,
            )
        };
        (status == ERROR_SUCCESS).then_some(Self(key))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the key was opened by `open` and is closed once, here.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(count: u32, time: u64) -> Vec<u8> {
        let mut record = vec![0u8; RECORD];
        record[RECORD_COUNT..RECORD_COUNT + 4].copy_from_slice(&count.to_le_bytes());
        record[RECORD_TIME..RECORD_TIME + 8].copy_from_slice(&time.to_le_bytes());
        record
    }

    #[test]
    fn names_are_rot13_both_ways_and_only_letters_move() {
        assert_eq!(
            rot13("Zvpebfbsg.Abgrcnq_8jrxlo3q8oojr!Ncc"),
            "Microsoft.Notepad_8wekyb3d8bbwe!App"
        );
        assert_eq!(rot13(r"P:\Gbbyf\é.rkr"), r"C:\Tools\é.exe");
        assert_eq!(rot13(&rot13("UEME_CTLSESSION")), "UEME_CTLSESSION");
    }

    #[test]
    fn a_record_gives_its_time_since_1970_and_its_count() {
        // 2021-01-01 00:00:00 UTC.
        const SECONDS: u64 = 1_609_459_200;
        let time = (SECONDS + EPOCH_GAP) * 10_000_000;
        assert_eq!(parse_record(&record(3, time)), Some((3, SECONDS)));
        assert_eq!(
            parse_record(&record(0, time)),
            Some((0, SECONDS)),
            "a count of 0 still ran"
        );
    }

    #[test]
    fn a_record_without_a_time_or_of_another_size_is_not_a_launch() {
        assert_eq!(parse_record(&record(5, 0)), None);
        assert_eq!(parse_record(&[0; 16]), None);
    }

    #[test]
    fn a_shortcut_is_named_after_what_it_opens() {
        assert_eq!(
            link_name(Path::new(r"C:\Recent\report.final.docx.lnk")),
            Some("report.final.docx")
        );
        assert_eq!(
            link_name(Path::new(r"C:\Recent\Downloads.LNK")),
            Some("Downloads")
        );
        assert_eq!(link_name(Path::new(r"C:\Recent\desktop.ini")), None);
    }

    #[test]
    fn an_id_is_a_path_only_when_it_looks_like_one() {
        assert_eq!(
            expanded_path(r"C:\Tools\a.exe").as_deref(),
            Some(r"C:\Tools\a.exe")
        );
        assert_eq!(
            expanded_path(r"\\host\a.exe").as_deref(),
            Some(r"\\host\a.exe")
        );
        assert_eq!(expanded_path("Microsoft.Notepad_8wekyb3d8bbwe!App"), None);
        assert_eq!(expanded_path("Microsoft.Windows.Explorer"), None);
    }

    #[test]
    fn a_known_folder_id_expands_to_that_folder() {
        // FOLDERID_System.
        let path = expanded_path(r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\notepad.exe").unwrap();
        assert!(
            path.to_lowercase().ends_with(r"\system32\notepad.exe"),
            "{path}"
        );
    }

    /// Reads the real history: only what a machine has, so it asserts the shape
    /// of the answer and prints counts, never names.
    #[test]
    fn the_real_history_is_well_formed() {
        let launches = launches();
        let files = files();
        eprintln!(
            "{} launches, {} files, tracks files: {}",
            launches.len(),
            files.len(),
            tracks_files()
        );
        assert!(launches.windows(2).all(|pair| pair[0].last >= pair[1].last));
        assert!(
            launches
                .iter()
                .all(|launch| !launch.id.starts_with("UEME_"))
        );
        assert!(files.iter().all(|file| !file.name.is_empty()));
    }
}
