use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::process;

use win_reveal::{
    dry_run_setting, Error, Revealer, ShellRevealer, DRY_RUN_VAR, EXIT_USAGE, USAGE,
};

/// Reveals nothing; prints what the shell would have been asked to select.
/// Lets the whole CLI be exercised end-to-end without opening Explorer windows.
struct DryRunRevealer;

impl Revealer for DryRunRevealer {
    fn reveal(&self, path: &Path) -> Result<(), Error> {
        println!("would reveal: {}", path.display());
        Ok(())
    }
}

/// Long help, reached only through `--help`/`-h` as the first argument.
/// The `usage:` line is the shared [`USAGE`] constant, so the error path cannot disagree with it.
fn help() -> String {
    format!(
        "reveal: open a path's containing folder in Windows Explorer and select it.

{USAGE}

  The path may be relative (resolved against the current directory) or absolute, and may
  contain spaces, CJK characters or commas. It must exist, and it must have a containing
  folder — a drive root like D:\\ has none.

options:
  -h, --help                        show this help and exit 0

settings (environment):
  REVEAL_DRY_RUN=1|true|yes         print the resolved path instead of touching Explorer
                                    (0|false|no|empty means off)
  REVEAL_TAB_POLICY=newtab|overwrite
                                    what to do when the folder is already open in a tab
                                    you are not looking at; newtab leaves that tab alone
                                    (default), overwrite navigates the visible tab to it

exit codes:
  0  the request was handed to the shell
  1  the shell call failed (stderr names the Win32 step and HRESULT)
  2  bad input or bad setting value (stderr gives the reason and the usage line)
"
    )
}

/// Reports a failure the way [`win_reveal::run`] reports a bad argument: reason, then usage.
fn fail(error: &Error) -> ! {
    let mut stderr = io::stderr();
    let _ = writeln!(stderr, "reveal: {error}");
    let _ = writeln!(stderr, "{USAGE}");
    let _ = stderr.flush();
    process::exit(EXIT_USAGE)
}

fn main() {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    // Checked before anything else, including the settings: someone lost in a wrong
    // REVEAL_DRY_RUN value still needs to be able to read the manual.
    if matches!(args.first(), Some(arg) if arg == "--help" || arg == "-h") {
        print!("{}", help());
        let _ = io::stdout().flush();
        return;
    }
    let revealer: Box<dyn Revealer> = match dry_run_setting(env::var_os(DRY_RUN_VAR)) {
        Ok(true) => Box::new(DryRunRevealer),
        Ok(false) => Box::new(ShellRevealer),
        Err(error) => fail(&error),
    };
    let cwd = match env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            eprintln!("reveal: cannot resolve working directory: {e}");
            process::exit(EXIT_USAGE);
        }
    };
    let mut stderr = io::stderr();
    let code = win_reveal::run(&args, &cwd, revealer.as_ref(), &mut stderr);
    let _ = stderr.flush();
    let _ = io::stdout().flush();
    process::exit(code);
}
