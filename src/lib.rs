//! Reveal a path in Windows Explorer: open the containing folder and select the item.
//!
//! This crate is Windows-only by design. The shell call is [`SHOpenFolderAndSelectItems`],
//! the one documented Win32 entry point whose contract is exactly "open this folder and
//! select these items". Paths are handed to the shell as UTF-16 through the shell
//! namespace, never through a command line, so spaces, CJK text and commas in names
//! cannot be mis-parsed.
//!
//! [`SHOpenFolderAndSelectItems`]: https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shopenfolderandselectitems

#[cfg(not(windows))]
compile_error!("reveal is Windows-only; build with a Windows target");

mod automation;
mod ffi;

#[cfg(test)]
mod crosscheck;

use std::borrow::Cow;
use std::cell::RefCell;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr;

use automation::Switch;

/// Process exit code when the reveal request was handed to the shell successfully.
pub const EXIT_OK: i32 = 0;
/// Process exit code when the shell call itself failed.
pub const EXIT_SHELL: i32 = 1;
/// Process exit code for bad command line input (wrong arg count, empty or missing path).
pub const EXIT_USAGE: i32 = 2;

const COINIT_APARTMENTTHREADED: u32 = 0x2;

/// Setting that chooses what happens when the target folder is already open in a tab the
/// user is not looking at. Accepted values: `newtab` (default) and `overwrite`.
const TAB_POLICY_VAR: &str = "REVEAL_TAB_POLICY";

/// Everything that can go wrong between the command line and the shell.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// No path argument was given.
    MissingArgument,
    /// More than one path argument was given.
    TooManyArguments(usize),
    /// The path argument was the empty string.
    EmptyArgument,
    /// The resolved path does not exist on disk.
    NotFound(PathBuf),
    /// The path is a drive root or otherwise has no containing folder to open.
    NoParent(PathBuf),
    /// The tab policy setting holds a value this tool does not understand.
    BadSetting { variable: &'static str, value: OsString },
    /// A Win32 shell call returned a failure HRESULT.
    Shell { step: &'static str, hresult: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MissingArgument => write!(f, "missing file path argument"),
            Error::TooManyArguments(n) => {
                write!(f, "expected exactly 1 file path argument, got {n}")
            }
            Error::EmptyArgument => write!(f, "file path argument is empty"),
            Error::NotFound(p) => write!(f, "path does not exist: {}", p.display()),
            Error::NoParent(p) => write!(f, "path has no containing folder: {}", p.display()),
            Error::BadSetting { variable, value } => write!(
                f,
                "{variable} must be 'newtab' or 'overwrite', got '{}'",
                value.to_string_lossy()
            ),
            Error::Shell { step, hresult } => {
                write!(f, "{step} failed (HRESULT 0x{hresult:08X})")
            }
        }
    }
}

/// The single injectable boundary between argument handling and the shell.
///
/// Production uses [`ShellRevealer`]; tests substitute a recorder so no Explorer
/// window is ever opened by the automated suite.
pub trait Revealer {
    /// Open the folder containing `path` and select `path`.
    fn reveal(&self, path: &Path) -> Result<(), Error>;
}

/// Reveals through the real Windows shell.
pub struct ShellRevealer;

impl Revealer for ShellRevealer {
    fn reveal(&self, path: &Path) -> Result<(), Error> {
        let switch = tab_policy(env::var_os(TAB_POLICY_VAR))?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // Every shell call below is COM-based and requires an initialized apartment;
        // a fresh CLI process has none.
        let hr = unsafe { ffi::CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED) };
        if hr < 0 {
            return Err(Error::Shell {
                step: "CoInitializeEx",
                hresult: hr as u32,
            });
        }
        let result = match select_in_already_open_view(path, switch) {
            Found::Done => Ok(()),
            // The folder is already in the user's window, only out of sight: bringing that
            // tab forward is enough, and asking the shell to open an open folder is what
            // produced a second tab for it.
            Found::Hidden => {
                ensure_selected(path, switch);
                Ok(())
            }
            Found::NotOpen => reveal_parsed(&wide).map(|()| ensure_selected(path, switch)),
        };
        unsafe { ffi::CoUninitialize() };
        result
    }
}

/// What the first look at the open Explorer tabs turned up.
enum Found {
    /// The item is selected in a tab the user can see.
    Done,
    /// A tab holds the folder but is not the one on screen, and the policy will not take
    /// over the visible tab.
    Hidden,
    /// No Explorer tab holds the folder, so it has to be opened.
    NotOpen,
}

/// Reads the tab policy out of an environment variable value.
///
/// Split out from [`ShellRevealer::reveal`] so the mapping is testable without process env.
fn tab_policy(raw: Option<OsString>) -> Result<Switch, Error> {
    let Some(value) = raw else {
        return Ok(Switch::NewTab);
    };
    match value.to_string_lossy().to_ascii_lowercase().as_str() {
        "newtab" => Ok(Switch::NewTab),
        "overwrite" => Ok(Switch::Overwrite),
        _ => Err(Error::BadSetting {
            variable: TAB_POLICY_VAR,
            value,
        }),
    }
}

/// Selecting into a tab that is already on screen is preferred over asking the shell to
/// open the folder, because that is the one path which is both visible and free: it adds no
/// tab and no window. A folder that is only open in a background tab counts as "not on
/// screen"; what happens to it next is `switch`'s job — see
/// [`automation::select_in_open_view`].
fn select_in_already_open_view(path: &Path, switch: Switch) -> Found {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Found::NotOpen;
    };
    // A view can exist but not list a just-created file yet; wait for Explorer's
    // asynchronous refresh before giving up and opening the folder anew.
    for _ in 0..10 {
        match automation::select_in_open_view(parent, name, switch) {
            Ok(hwnd) => {
                automation::bring_to_front(hwnd);
                return Found::Done;
            }
            // Only the item is missing, the folder is on screen: after the wait, fall
            // through to the open call, which is also what refreshes the item list.
            Err(automation::Miss::ItemNotListed) => {
                std::thread::sleep(std::time::Duration::from_millis(200))
            }
            Err(automation::Miss::Hidden) => return Found::Hidden,
            Err(automation::Miss::NoView) => return Found::NotOpen,
        }
    }
    Found::NotOpen
}

/// Some Windows 11 shells open the folder but drop the requested selection when they
/// have to create a new window or tab, and a window they create can land behind the
/// caller's own — either way the user sees nothing happen. Re-apply the selection through
/// the folder view of the window that is now open and raise it. With the new-tab policy,
/// also ask the shell to switch to the folder's tab when the open it produced is a
/// background one. Best effort: a failure here leaves the folder open, which is still the
/// documented result of the shell call above.
fn ensure_selected(path: &Path, switch: Switch) {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let mut asked = false;
    for _ in 0..15 {
        if let Some(hwnd) = automation::front_window_for(parent) {
            automation::bring_to_front(hwnd);
            match automation::select_in_open_view(parent, name, switch) {
                Ok(_) => return,
                // Reached once, and only when the folder is open out of sight: the shell put
                // it in the user's existing window and left their own tab in front.
                Err(automation::Miss::Hidden) if !asked && switch == Switch::NewTab => {
                    asked = true;
                    automation::activate_tab_by_path(parent);
                }
                Err(_) => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// Records every path it is asked to reveal instead of touching the shell.
#[derive(Default)]
pub struct RecordingRevealer(pub RefCell<Vec<PathBuf>>);

impl RecordingRevealer {
    pub fn recorded(&self) -> Vec<PathBuf> {
        self.0.borrow().clone()
    }
}

impl Revealer for RecordingRevealer {
    fn reveal(&self, path: &Path) -> Result<(), Error> {
        self.0.borrow_mut().push(path.to_path_buf());
        Ok(())
    }
}

/// Spell the argument the way the shell namespace means it, when it differs from Win32.
///
/// Three spellings need it, all measured on build 26100:
///
/// * `\\?\C:\a.txt` is a valid Win32 path, so the existence check below accepts it, but
///   `SHParseDisplayName` answers `E_INVALIDARG` for it — without the rewrite the tool would
///   report a shell failure for a path it just verified exists. Device-namespace forms
///   (`\\?\Device\...`) have no shell equivalent and are left alone: they fail where they
///   fail, with the HRESULT that says so.
/// * `C:a.txt` means "relative to the current directory *of drive C:*", which Explorer takes
///   from its own process, not from the caller's. Anchoring it to the drive root keeps one
///   path meaning one file.
/// * `\\server\share` needs nothing, and neither does an already ordinary absolute path.
fn normalize(path: &Path) -> Cow<'_, Path> {
    let mut components = path.components();
    let head = match components.next() {
        Some(Component::Prefix(prefix)) if !path.has_root() => {
            // A drive letter or UNC root without a `\` after it: rebuild the rooted form.
            match prefix.kind() {
                Prefix::Disk(disk) | Prefix::VerbatimDisk(disk) => {
                    Some(OsString::from(format!("{}:\\", disk as char)))
                }
                _ => None,
            }
        }
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(disk) => Some(OsString::from(format!("{}:\\", disk as char))),
            Prefix::VerbatimUNC(server, share) => {
                let mut head = OsString::from(r"\\");
                head.push(server);
                head.push("\\");
                head.push(share);
                head.push("\\");
                Some(head)
            }
            _ => None,
        },
        _ => None,
    };
    let Some(mut rewritten) = head else {
        return Cow::Borrowed(path);
    };
    // The head already carries its separator, so drop the remainder's leading one to keep a
    // single separator at the join instead of `C:\\Windows`.
    let rest = components.as_path();
    let rest = rest.strip_prefix("\\").unwrap_or(rest);
    rewritten.push(rest.as_os_str());
    Cow::Owned(PathBuf::from(rewritten))
}

/// Resolve the raw command line argument to the absolute path that will be revealed.
///
/// Relative input is resolved against `cwd` because Explorer would otherwise resolve it
/// against its own process directory, which is unrelated to the caller's.
pub fn resolve(raw: &std::ffi::OsStr, cwd: &Path) -> Result<PathBuf, Error> {
    if raw.is_empty() {
        return Err(Error::EmptyArgument);
    }
    let path = normalize(Path::new(raw));
    let joined = if path.is_relative() {
        cwd.join(&*path)
    } else {
        path.to_path_buf()
    };
    // Lexical only: drops trailing separators and `.` components, which would otherwise
    // make a trailing-separator file path fail the existence check below.
    let abs: PathBuf = joined.components().collect();
    if !abs.exists() {
        return Err(Error::NotFound(abs));
    }
    match abs.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(abs),
        _ => Err(Error::NoParent(abs)),
    }
}

/// Full CLI entry point: parse `args`, reveal, and report to `stderr`.
/// Returns the process exit code.
pub fn run(
    args: &[OsString],
    cwd: &Path,
    revealer: &dyn Revealer,
    stderr: &mut dyn Write,
) -> i32 {
    let result = match args.len() {
        0 => Err(Error::MissingArgument),
        1 => resolve(&args[0], cwd).and_then(|p| revealer.reveal(&p).map(|()| p)),
        n => Err(Error::TooManyArguments(n)),
    };
    match result {
        Ok(_) => EXIT_OK,
        Err(e) => {
            let code = match e {
                Error::Shell { .. } => EXIT_SHELL,
                _ => EXIT_USAGE,
            };
            let _ = writeln!(stderr, "reveal: {e}");
            if code == EXIT_USAGE {
                let _ = writeln!(stderr, "usage: reveal <file-or-folder-path>");
            }
            code
        }
    }
}

fn reveal_parsed(wide: &[u16]) -> Result<(), Error> {
    unsafe {
        let mut pidl = ptr::null_mut();
        let hr = ffi::SHParseDisplayName(wide.as_ptr(), ptr::null(), &mut pidl, 0, ptr::null_mut());
        if hr < 0 || pidl.is_null() {
            return Err(Error::Shell {
                step: "SHParseDisplayName",
                hresult: hr as u32,
            });
        }
        // Split the parsed item into (parent folder, child) as the API requires:
        // truncate the full IDLIST in place to the parent and keep the last element
        // as the relative child to select.
        let child = ffi::ILFindLastID(pidl);
        ffi::ILRemoveLastID(pidl);
        let child_ptr: *const std::os::raw::c_void = child;
        let hr = ffi::SHOpenFolderAndSelectItems(pidl, 1, &child_ptr, 0);
        ffi::ILFree(pidl);
        if hr < 0 {
            return Err(Error::Shell {
                step: "SHOpenFolderAndSelectItems",
                hresult: hr as u32,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "reveal-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_with(args: &[&str], cwd: &Path, rec: &RecordingRevealer) -> (i32, String) {
        let mut err = Vec::new();
        let owned: Vec<OsString> = args.iter().map(OsString::from).collect();
        let code = run(&owned, cwd, rec, &mut err);
        (code, String::from_utf8_lossy(&err).into_owned())
    }

    #[test]
    fn missing_argument_is_usage_error() {
        let rec = RecordingRevealer::default();
        let (code, err) = run_with(&[], Path::new("C:\\"), &rec);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("missing file path argument"), "{err}");
        assert!(err.contains("usage: reveal"), "{err}");
        assert!(rec.recorded().is_empty());
    }

    #[test]
    fn extra_arguments_are_usage_error() {
        let rec = RecordingRevealer::default();
        let (code, err) = run_with(&["a.txt", "b.txt"], Path::new("C:\\"), &rec);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("expected exactly 1 file path argument, got 2"), "{err}");
        assert!(rec.recorded().is_empty());
    }

    #[test]
    fn empty_argument_is_usage_error() {
        let rec = RecordingRevealer::default();
        let (code, err) = run_with(&[""], Path::new("C:\\"), &rec);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("empty"), "{err}");
        assert!(rec.recorded().is_empty());
    }

    #[test]
    fn nonexistent_path_is_usage_error_and_not_revealed() {
        let dir = temp_dir("missing");
        let rec = RecordingRevealer::default();
        let missing = dir.join("nope.txt");
        let (code, err) = run_with(&[missing.to_str().unwrap()], &dir, &rec);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("path does not exist"), "{err}");
        assert!(rec.recorded().is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn relative_path_is_resolved_against_given_cwd() {
        let dir = temp_dir("relative");
        let file = dir.join("inner.txt");
        fs::write(&file, "x").unwrap();
        let rec = RecordingRevealer::default();
        let (code, _) = run_with(&["inner.txt"], &dir, &rec);
        assert_eq!(code, EXIT_OK);
        assert_eq!(rec.recorded(), vec![file]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn spaces_and_cjk_survive_unchanged() {
        let dir = temp_dir("cjk");
        let nested = dir.join("some dir 中文目录");
        fs::create_dir_all(&nested).unwrap();
        let file = nested.join("空格 文件 中文.txt");
        fs::write(&file, "x").unwrap();
        let rec = RecordingRevealer::default();
        let (code, _) = run_with(&[file.to_str().unwrap()], &dir, &rec);
        assert_eq!(code, EXIT_OK);
        assert_eq!(rec.recorded(), vec![file]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn comma_and_ampersand_in_name_survive_unchanged() {
        let dir = temp_dir("meta");
        let file = dir.join("a,b & c.txt");
        fs::write(&file, "x").unwrap();
        let rec = RecordingRevealer::default();
        let (code, _) = run_with(&[file.to_str().unwrap()], &dir, &rec);
        assert_eq!(code, EXIT_OK);
        assert_eq!(rec.recorded(), vec![file]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn directory_input_is_revealed_as_itself() {
        let dir = temp_dir("dirinput");
        let inner = dir.join("subdir");
        fs::create_dir_all(&inner).unwrap();
        let rec = RecordingRevealer::default();
        let (code, _) = run_with(&[inner.to_str().unwrap()], &dir, &rec);
        assert_eq!(code, EXIT_OK);
        assert_eq!(rec.recorded(), vec![inner]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trailing_separator_and_dotdot_are_accepted() {
        let dir = temp_dir("dotdot");
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let file = nested.join("f.txt");
        fs::write(&file, "x").unwrap();

        let rec = RecordingRevealer::default();
        let with_slash = format!("{}\\", file.to_str().unwrap());
        let (code, _) = run_with(&[&with_slash], &dir, &rec);
        assert_eq!(code, EXIT_OK);
        assert_eq!(rec.recorded(), vec![file]);

        let rec2 = RecordingRevealer::default();
        let with_dotdot = dir.join("nested").join("..").join("nested").join("f.txt");
        let (code2, _) = run_with(&[with_dotdot.to_str().unwrap()], &dir, &rec2);
        assert_eq!(code2, EXIT_OK);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drive_root_has_no_parent_and_is_rejected() {
        let rec = RecordingRevealer::default();
        let (code, err) = run_with(&["C:\\"], Path::new("C:\\"), &rec);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("no containing folder"), "{err}");
        assert!(rec.recorded().is_empty());
    }

    #[test]
    fn shell_failure_maps_to_exit_code_one() {
        struct Failing;
        impl Revealer for Failing {
            fn reveal(&self, _path: &Path) -> Result<(), Error> {
                Err(Error::Shell {
                    step: "SHOpenFolderAndSelectItems",
                    hresult: 0x8000_4005,
                })
            }
        }
        let dir = temp_dir("shellfail");
        let file = dir.join("f.txt");
        fs::write(&file, "x").unwrap();
        let mut err = Vec::new();
        let code = run(
            &[OsString::from(file.as_os_str())],
            &dir,
            &Failing,
            &mut err,
        );
        assert_eq!(code, EXIT_SHELL);
        let err = String::from_utf8_lossy(&err);
        assert!(err.contains("SHOpenFolderAndSelectItems failed"), "{err}");
        assert!(err.contains("0x80004005"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_keeps_absolute_path_verbatim() {
        let dir = temp_dir("abs");
        let file = dir.join("f.txt");
        fs::write(&file, "x").unwrap();
        assert_eq!(resolve(file.as_os_str(), Path::new("C:\\elsewhere")).unwrap(), file);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn verbatim_disk_path_is_rewritten_before_the_shell_sees_it() {
        let dir = temp_dir("verbatim");
        let file = dir.join("目标 file.txt");
        fs::write(&file, "x").unwrap();
        let extended = PathBuf::from(OsString::from(format!(r"\\?\{}", file.display())));
        assert_eq!(&*normalize(&extended), file.as_path());
        // The extended spelling passes the existence check, so it has to reach the reveal.
        let rec = RecordingRevealer::default();
        let (code, _) = run_with(&[extended.to_str().unwrap()], &dir, &rec);
        assert_eq!(code, EXIT_OK);
        assert_eq!(rec.recorded(), vec![file]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn verbatim_unc_path_is_rewritten_to_its_unc_form() {
        let unc = Path::new(r"\\?\UNC\fileserver\share\dir\报告, v2.txt");
        assert_eq!(&*normalize(unc), Path::new(r"\\fileserver\share\dir\报告, v2.txt"));
        // A bare extended drive root keeps exactly one separator.
        assert_eq!(&*normalize(Path::new(r"\\?\C:\")), Path::new(r"C:\"));
    }

    #[test]
    fn drive_relative_path_is_anchored_before_the_shell_sees_it() {
        // `C:a.txt` is relative to the current directory *of drive C:* — state Explorer keeps
        // for itself, not the caller's. Anchoring it to the drive root is the only way one
        // path argument keeps meaning one file.
        assert_eq!(&*normalize(Path::new("C:Windows\\notepad.exe")), Path::new(r"C:\Windows\notepad.exe"));
        assert_eq!(&*normalize(Path::new("C:a.txt")), Path::new(r"C:\a.txt"));

        let dir = temp_dir("drive-relative");
        let file = dir.join("报告 file.txt");
        fs::write(&file, "x").unwrap();
        let absolute = file.display().to_string();
        assert_eq!(&absolute.as_bytes()[1..3], b":\\", "{absolute} is not a drive-lettered path");
        let drive_relative = format!("{}:{}", &absolute[..1], &absolute[3..]);
        let rec = RecordingRevealer::default();
        let (code, _) = run_with(&[&drive_relative], &dir, &rec);
        assert_eq!(code, EXIT_OK);
        assert_eq!(rec.recorded(), vec![file]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_the_namespace_cannot_represent_are_left_as_they_are() {
        // Device-namespace and other whole-path verbatim forms have no shell equivalent, and
        // ordinary paths need nothing: rewriting these would invent a different target.
        for untouched in [
            r"\\?\Device\Harddisk0\Partition1",
            r"\\.\GLOBAL\RPC",
            r"C:\Windows\notepad.exe",
            r"\\server\share\a.txt",
            r"relative\name.txt",
        ] {
            assert_eq!(
                &*normalize(Path::new(untouched)),
                Path::new(untouched),
                "{untouched}"
            );
        }
    }

    #[test]
    fn tab_policy_defaults_to_a_new_tab() {
        assert_eq!(tab_policy(None).unwrap(), Switch::NewTab);
        assert_eq!(tab_policy(Some(OsString::from("newtab"))).unwrap(), Switch::NewTab);
    }

    #[test]
    fn tab_policy_reads_overwrite_case_insensitively() {
        assert_eq!(tab_policy(Some(OsString::from("Overwrite"))).unwrap(), Switch::Overwrite);
        assert_eq!(tab_policy(Some(OsString::from("OVERWRITE"))).unwrap(), Switch::Overwrite);
    }

    #[test]
    fn tab_policy_rejects_unknown_spelling_with_both_values_named() {
        let error = tab_policy(Some(OsString::from("sideways"))).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("REVEAL_TAB_POLICY must be 'newtab' or 'overwrite'"), "{message}");
        assert!(message.contains("sideways"), "{message}");
    }
}
