use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::process;

use win_reveal::{Error, Revealer, ShellRevealer, EXIT_USAGE};

/// Reveals nothing; prints what the shell would have been asked to select.
/// Lets the whole CLI be exercised end-to-end without opening Explorer windows.
struct DryRunRevealer;

impl Revealer for DryRunRevealer {
    fn reveal(&self, path: &Path) -> Result<(), Error> {
        println!("would reveal: {}", path.display());
        Ok(())
    }
}

fn main() {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let cwd = match env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            eprintln!("reveal: cannot resolve working directory: {e}");
            process::exit(EXIT_USAGE);
        }
    };
    let dry_run = env::var_os("REVEAL_DRY_RUN").is_some();
    let revealer: Box<dyn Revealer> = if dry_run {
        Box::new(DryRunRevealer)
    } else {
        Box::new(ShellRevealer)
    };
    let mut stderr = io::stderr();
    let code = win_reveal::run(&args, &cwd, revealer.as_ref(), &mut stderr);
    let _ = stderr.flush();
    let _ = io::stdout().flush();
    process::exit(code);
}
