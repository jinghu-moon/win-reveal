// The crate root's unit tests, in a child module of that root: a child sees the root's private
// items, so `use super::*` keeps working here while `tests/` — a separate crate against the
// public surface — would force internals to become `pub` just to be testable.

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

#[test]
fn dry_run_is_off_unless_a_value_says_otherwise() {
    // An exported variable that can only ever turn itself on is the footgun: a session that
    // set it once can never make a real reveal again. So the off spellings are load-bearing.
    for off in [None, Some(""), Some("0"), Some("false"), Some("NO"), Some("False")] {
        assert!(!dry_run_setting(off.map(OsString::from)).unwrap(), "{off:?}");
    }
    for on in [Some("1"), Some("true"), Some("YES"), Some("True")] {
        assert!(dry_run_setting(on.map(OsString::from)).unwrap(), "{on:?}");
    }
}

#[test]
fn dry_run_rejects_a_value_that_means_neither_side() {
    // Not even a silent default: the whole point of the variable is that what it prints is
    // what the shell would have done.
    let error = dry_run_setting(Some(OsString::from("maybe"))).unwrap_err();
    assert_eq!(
        error,
        Error::BadSetting {
            variable: DRY_RUN_VAR,
            value: OsString::from("maybe"),
            allowed: "'1'/'0', 'true'/'false' or 'yes'/'no'",
        }
    );
    let message = error.to_string();
    assert!(
        message.contains("REVEAL_DRY_RUN must be '1'/'0', 'true'/'false' or 'yes'/'no', got 'maybe'"),
        "{message}"
    );
}

#[test]
fn every_usage_error_ends_with_the_usage_constant() {
    // --help quotes USAGE too, so this is what keeps the two texts pointing at one line.
    let dir = temp_dir("usage");
    let rec = RecordingRevealer::default();
    for args in [&[] as &[&str], &["a.txt", "b.txt"], &[""]] {
        let (code, err) = run_with(args, &dir, &rec);
        assert_eq!(code, EXIT_USAGE, "{args:?}");
        assert_eq!(err.lines().last(), Some(USAGE), "{args:?}");
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn error_type_plugs_into_a_callers_error_chain() {
    // A host writing `fn f() -> Result<(), Box<dyn Error>>` can only take `?` from
    // `reveal` if our error type is itself an error type. Nothing else in the suite would
    // notice the impl going away, because no test names it.
    fn takes<E: std::error::Error>() {}
    takes::<Error>();
}

#[test]
fn an_apartment_the_host_already_owns_is_not_reported_as_failure() {
    // Stands in for a GUI host that initialized this thread as MTA before calling us:
    // asking for STA on top of that is RPC_E_CHANGED_MODE, a negative HRESULT for a call
    // that is perfectly usable. Refusing it would break every such host, and uninitializing
    // would tear down the host's apartment rather than ours.
    let host = unsafe { ffi::CoInitializeEx(ptr::null(), 0) }; // 0 = COINIT_MULTITHREADED
    assert!(host == 0 || host == 1, "MTA init failed: {host:#010x}");
    assert_eq!(
        init_apartment(),
        Ok(false),
        "a host-owned apartment must be adopted, not refused"
    );
    unsafe { ffi::CoUninitialize() };
}

#[test]
fn the_revealers_can_be_shared_across_threads() {
    // A host hands a revealer to whatever thread makes the call, usually behind
    // `Arc<dyn Revealer + Send + Sync>`. A `RefCell` inside the recorder made that not
    // compile, and no other test names the trait bounds.
    fn takes<T: Send + Sync + Revealer>() {}
    takes::<RecordingRevealer>();
    takes::<ShellRevealer>();
}
