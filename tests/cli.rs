//! End-to-end tests against the real binary.
//!
//! Every case here either fails before reaching the shell (so no Explorer window can
//! open), reaches it only far enough to reject a setting, or runs with the dry-run
//! setting on, which swaps the shell boundary for a printer. The automated suite
//! therefore never opens an Explorer window.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_reveal")
}

/// A command with both settings cleared, so a value exported in the shell that runs cargo
/// cannot silently change what a test observes. Each test states what it means.
fn command(args: &[&str], cwd: Option<&Path>) -> Command {
    let mut cmd = Command::new(bin());
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.env_remove("REVEAL_DRY_RUN");
    cmd.env_remove("REVEAL_TAB_POLICY");
    cmd
}

fn run(args: &[&str], cwd: Option<&Path>, dry_run: bool) -> Output {
    let mut cmd = command(args, cwd);
    if dry_run {
        cmd.env("REVEAL_DRY_RUN", "1");
    }
    cmd.output().expect("reveal binary should run")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn fixture(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "reveal-cli-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn no_arguments_exits_two_with_usage() {
    let out = run(&[], None, true);
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    assert!(err.contains("missing file path argument"), "{err}");
    assert!(err.contains("usage: reveal"), "{err}");
}

#[test]
fn two_arguments_exit_two_with_usage() {
    let out = run(&["a.txt", "b.txt"], None, true);
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    assert!(err.contains("expected exactly 1 file path argument, got 2"), "{err}");
}

#[test]
fn empty_argument_exits_two() {
    let out = run(&[""], None, true);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("empty"), "{}", text(&out.stderr));
}

#[test]
fn nonexistent_path_exits_two_and_names_the_path() {
    let dir = fixture("missing");
    let missing = dir.join("nope.txt");
    let out = run(&[missing.to_str().unwrap()], Some(&dir), true);
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    assert!(err.contains("path does not exist"), "{err}");
    assert!(err.contains("nope.txt"), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn nonexistent_path_without_dry_run_also_exits_two_before_touching_shell() {
    let dir = fixture("missing-real");
    let missing = dir.join("nope.txt");
    let out = run(&[missing.to_str().unwrap()], Some(&dir), false);
    assert_eq!(out.status.code(), Some(2));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dry_run_reports_exact_path_with_spaces_and_cjk() {
    let dir = fixture("cjk");
    let nested = dir.join("some dir 中文目录");
    std::fs::create_dir_all(&nested).unwrap();
    let file = nested.join("空格 文件 中文.txt");
    std::fs::write(&file, "x").unwrap();

    let out = run(&[file.to_str().unwrap()], Some(&dir), true);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.contains(&format!("would reveal: {}", file.display())), "{stdout}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dry_run_reports_exact_path_with_comma_and_ampersand() {
    let dir = fixture("meta");
    let file = dir.join("a,b & c.txt");
    std::fs::write(&file, "x").unwrap();
    let out = run(&[file.to_str().unwrap()], Some(&dir), true);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("a,b & c.txt"), "{}", text(&out.stdout));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dry_run_resolves_relative_path_against_process_cwd() {
    let dir = fixture("relative");
    let file = dir.join("inner.txt");
    std::fs::write(&file, "x").unwrap();
    let out = run(&["inner.txt"], Some(&dir), true);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.contains(&format!("would reveal: {}", file.display())), "{stdout}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dry_run_accepts_directory_and_trailing_separator() {
    let dir = fixture("dir");
    let inner = dir.join("subdir");
    std::fs::create_dir_all(&inner).unwrap();

    let out = run(&[inner.to_str().unwrap()], Some(&dir), true);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));

    let with_sep = format!("{}\\", inner.to_str().unwrap());
    let out = run(&[&with_sep], Some(&dir), true);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn drive_root_is_rejected_with_clear_message() {
    let out = run(&["C:\\"], None, true);
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    assert!(err.contains("no containing folder"), "{err}");
}

#[test]
fn success_writes_nothing_to_stderr() {
    let dir = fixture("quiet");
    let file = dir.join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let out = run(&[file.to_str().unwrap()], Some(&dir), true);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty(), "{}", text(&out.stderr));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unknown_tab_policy_exits_two_before_touching_the_shell() {
    let dir = fixture("badpolicy");
    let file = dir.join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let out = command(&[], Some(&dir))
        .arg(&file)
        .env("REVEAL_TAB_POLICY", "sideways")
        .output()
        .expect("reveal binary should run");
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    assert!(err.contains("REVEAL_TAB_POLICY must be 'newtab' or 'overwrite'"), "{err}");
    assert!(err.contains("sideways"), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn help_goes_to_stdout_and_exits_zero() {
    for flag in ["--help", "-h"] {
        let out = run(&[flag], None, false);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let stdout = text(&out.stdout);
        assert!(stdout.starts_with("reveal: open a path's containing folder"), "{stdout}");
        // The two settings and the three exit codes are the parts a user can actually get wrong.
        assert!(stdout.contains("REVEAL_DRY_RUN"), "{stdout}");
        assert!(stdout.contains("REVEAL_TAB_POLICY"), "{stdout}");
        assert!(stdout.contains("newtab"), "{stdout}");
        assert!(out.stderr.is_empty(), "{}", text(&out.stderr));
    }
}

#[test]
fn help_and_the_error_path_quote_the_same_usage_line() {
    let help = run(&["--help"], None, false);
    let usage = text(&help.stdout)
        .lines()
        .find(|line| line.starts_with("usage: "))
        .expect("help states a usage line")
        .to_owned();
    let out = run(&[], None, false);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        text(&out.stderr).lines().last(),
        Some(usage.as_str()),
        "usage in help and in errors must be one string"
    );
}

#[test]
fn help_only_helps_as_the_first_argument() {
    let out = run(&["--helpme"], None, true);
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    assert!(err.contains("path does not exist"), "{err}");
    assert!(err.contains("--helpme"), "{err}");

    let out = run(&["Cargo.toml", "--help"], None, true);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr).contains("expected exactly 1 file path argument, got 2"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn help_survives_a_broken_setting() {
    // Reading the manual is the way out of a bad value, so it must not need a good value first.
    let out = command(&["--help"], None)
        .env("REVEAL_DRY_RUN", "maybe")
        .env("REVEAL_TAB_POLICY", "sideways")
        .output()
        .expect("reveal binary should run");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).starts_with("reveal: open a path's"), "{}", text(&out.stdout));
}

#[test]
fn a_bad_dry_run_value_is_refused_by_name() {
    let dir = fixture("dryrun-bad");
    let file = dir.join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let out = command(&[], Some(&dir))
        .arg(&file)
        .env("REVEAL_DRY_RUN", "maybe")
        .output()
        .expect("reveal binary should run");
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    assert!(
        err.contains("REVEAL_DRY_RUN must be '1'/'0', 'true'/'false' or 'yes'/'no', got 'maybe'"),
        "{err}"
    );
    assert!(err.contains("usage: reveal"), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dry_run_off_spellings_reach_the_shell_instead_of_printing() {
    let dir = fixture("dryrun-off");
    let file = dir.join("f.txt");
    std::fs::write(&file, "x").unwrap();
    // Proof that the switch really went off: with dry run on the tab policy is never read,
    // so a rejected policy value here can only come from the shell revealer — and it fails
    // before any Explorer window can open.
    for off in ["0", "false", "NO", ""] {
        let out = command(&[], Some(&dir))
            .arg(&file)
            .env("REVEAL_DRY_RUN", off)
            .env("REVEAL_TAB_POLICY", "sideways")
            .output()
            .expect("reveal binary should run");
        assert_eq!(out.status.code(), Some(2), "REVEAL_DRY_RUN={off:?}");
        assert!(out.stdout.is_empty(), "REVEAL_DRY_RUN={off:?} printed a dry-run line");
        assert!(text(&out.stderr).contains("REVEAL_TAB_POLICY"), "REVEAL_DRY_RUN={off:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dry_run_on_spellings_skip_the_shell_entirely() {
    let dir = fixture("dryrun-on");
    let file = dir.join("f.txt");
    std::fs::write(&file, "x").unwrap();
    for on in ["1", "true", "YES"] {
        let out = command(&[], Some(&dir))
            .arg(&file)
            .env("REVEAL_DRY_RUN", on)
            // Even an invalid policy is unread: the dry-run branch never constructs the
            // shell revealer, which is the only place that looks at it.
            .env("REVEAL_TAB_POLICY", "sideways")
            .output()
            .expect("reveal binary should run");
        assert_eq!(out.status.code(), Some(0), "REVEAL_DRY_RUN={on:?}: {}", text(&out.stderr));
        assert!(text(&out.stdout).contains("would reveal:"), "REVEAL_DRY_RUN={on:?}");
        assert!(out.stderr.is_empty(), "REVEAL_DRY_RUN={on:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
