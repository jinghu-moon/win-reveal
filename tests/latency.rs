//! Opt-in latency baseline for the real shell path.
//!
//! Opens Explorer windows, so it is excluded from the default suite. Run with:
//!
//! ```text
//! REVEAL_LATENCY_TEST=1 cargo test --test latency -- --ignored --nocapture
//! ```
//!
//! Every row is wall clock measured from outside the process, because that is what a caller
//! feels: spawn + argument handling + COM init + the open-view probe + the shell call + the
//! bounded compensation wait. Rows A and B are the floors (neither reaches COM), so
//! subtracting them from C..F leaves the shell work exposed without shipping any
//! instrumentation inside the binary.
//!
//! Nothing is asserted about absolute time: a baseline that goes red on a busy machine
//! teaches nothing. It prints min/median/max over a handful of runs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Closes the Explorer frames whose tabs are *all* fixtures.
///
/// Same rule as the UI test: a frame that also hosts one of the user's own tabs is left
/// alone, because closing the frame would close those too. Whatever survives is counted and
/// reported, so the run says out loud what it left on the desktop.
const CLOSE_FIXTURE_FRAMES: &str = "\
Add-Type -Name Lat -Namespace Reveal -MemberDefinition \
'[DllImport(\"user32.dll\")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);'; \
$shell=New-Object -ComObject Shell.Application; \
$frames=@{}; \
for($i=0; $i -lt $shell.Windows().Count; $i++){ \
  $w=$shell.Windows().Item($i); \
  try{ \
    $h=[IntPtr]$w.HWND; $p=$w.Document.Folder.Self.Path; \
    if(-not $frames.ContainsKey($h)){ $frames[$h]=@() }; $frames[$h]+=$p \
  }catch{ } \
}; \
foreach($h in $frames.Keys){ \
  $owned=$true; \
  foreach($p in $frames[$h]){ if(-not $p.StartsWith($env:REVEAL_LAT_ROOT)){ $owned=$false } }; \
  if($owned){ [Reveal.Lat]::PostMessage($h,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null } \
}";

/// How many `Shell.Windows` entries (one per tab) and distinct frame HWNDs the probe sees.
/// The tab count is the input its cost scales with.
const COUNT_VIEWS: &str = "\
$shell=New-Object -ComObject Shell.Application; \
$tabs=$shell.Windows().Count; \
$set=@{}; \
for($i=0; $i -lt $tabs; $i++){ try{ $set[[IntPtr]$shell.Windows().Item($i).HWND]=0 }catch{ } }; \
\"$tabs $($set.Count)\"";

/// `("<tabs>", "<frames>")`.
fn views(root: &Path) -> (String, String) {
    powershell(COUNT_VIEWS, root)
        .split_once(' ')
        .map(|(tabs, frames)| (tabs.to_owned(), frames.to_owned()))
        .unwrap_or_else(|| ("?".to_owned(), "?".to_owned()))
}

fn powershell(script: &str, root: &Path) -> String {
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env("REVEAL_LAT_ROOT", root)
        .output()
        .expect("powershell should run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// One row: `case min median max ms n=<runs> <note>`.
fn row(case: &str, note: &str, samples: &[u128]) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    let median = if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2
    };
    println!(
        "{:<3} {:>6} {:>6} {:>6} ms  n={:<2} {}",
        case,
        sorted[0],
        median,
        sorted[sorted.len() - 1],
        sorted.len(),
        note
    );
}

/// Runs the binary once and returns `(exit code, wall ms)`.
///
/// Both settings are stated rather than inherited: a `REVEAL_TAB_POLICY` or `REVEAL_DRY_RUN`
/// exported in the shell running cargo would silently time a different path.
fn spawn(path: &Path, dry_run: Option<&str>) -> (i32, u128) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_reveal"));
    cmd.arg(path);
    match dry_run {
        Some(value) => cmd.env("REVEAL_DRY_RUN", value),
        None => cmd.env_remove("REVEAL_DRY_RUN"),
    };
    cmd.env_remove("REVEAL_TAB_POLICY");
    let started = Instant::now();
    let out = cmd.output().expect("reveal binary should run");
    (
        out.status.code().unwrap_or(-1),
        started.elapsed().as_millis(),
    )
}

/// Times `count` calls that must all succeed, and returns the samples.
fn timed(count: usize, path: &Path, dry_run: Option<&str>, what: &str) -> Vec<u128> {
    (0..count)
        .map(|_| {
            let (code, ms) = spawn(path, dry_run);
            assert_eq!(code, 0, "{what} failed with exit code {code}");
            ms
        })
        .collect()
}

#[test]
#[ignore = "opens real Explorer windows; run with --ignored and REVEAL_LATENCY_TEST=1"]
fn latency_baseline() {
    if std::env::var_os("REVEAL_LATENCY_TEST").is_none() {
        println!("skipping latency baseline; set REVEAL_LATENCY_TEST=1 to opt in");
        return;
    }

    let root = std::env::temp_dir().join(format!("reveal-latency-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let fixture = |tag: &str, name: &str| {
        let dir = root.join(tag);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(name);
        std::fs::write(&file, "x").unwrap();
        file
    };

    println!("fixture root: {}", root.display());
    let (tabs_before, frames_before) = views(&root);
    println!("start: {tabs_before} tab(s) in {frames_before} frame(s) for the probe to walk");
    println!("min median max");
    // A: no argument handling beyond the flag, no resolve, no COM.
    row("A", "process floor: --help", &timed(15, Path::new("--help"), None, "--help"));
    // B: adds resolve() and the existence check, but still no COM.
    let plain = fixture("plain", "f.txt");
    row("B", "dry run: resolve + exists()", &timed(15, &plain, Some("1"), "dry run"));

    // C: folder open nowhere, so the shell has to create the tab or window. Four distinct
    // fixtures, because a second reveal of one folder is warm by definition — the last three
    // end up as the background tabs row F needs.
    let cold: Vec<PathBuf> = ["cold1", "cold2", "cold3", "cold4"]
        .into_iter()
        .map(|tag| fixture(tag, "target.txt"))
        .collect();
    let cold_ms: Vec<u128> = cold
        .iter()
        .map(|file| {
            let ms = spawn(file, None).1;
            // Let the window Explorer created settle before timing the next one.
            std::thread::sleep(Duration::from_millis(1500));
            ms
        })
        .collect();
    row("C", "cold: folder not open anywhere", &cold_ms);
    // The four are printed individually because the tail is the interesting part: the first
    // real shell call of a session also pays for activating the Shell.Application COM server.
    println!("    raw: {cold_ms:?} ms in cold1..cold4 order");

    // D: the cheapest real path — the folder is already the visible tab and the item is
    // already in its list, so the probe selects and nothing is opened. cold4 is on screen
    // because C revealed it last.
    let visible = &cold[3];
    row(
        "D",
        "warm: visible tab, same file again",
        &timed(8, visible, None, "warm reveal"),
    );

    // E: what happens to anything that reveals a file it just wrote. The folder is on screen
    // but its list does not hold the item yet, so the probe waits for Explorer's refresh in
    // 200 ms steps before falling through to the shell call.
    let late: Vec<u128> = (0..3)
        .map(|n| {
            let file = visible.parent().unwrap().join(format!("just-written-{n}.txt"));
            std::fs::write(&file, "x").unwrap();
            let (_, ms) = spawn(&file, None);
            ms
        })
        .collect();
    row("E", "warm: item just created, refresh wait", &late);

    // F: the folder is open but in a background tab — the case REVEAL_TAB_POLICY exists for,
    // and the one the measured shell defect makes expensive. Each sample takes a different
    // fixture folder and is preceded by an unmeasured reveal of `visible`, because the
    // previous timed call left *its* folder on screen: without the reset, the second and
    // third samples would quietly be row D. With the default policy each of these can add a
    // duplicate tab, which is part of what is being paid for.
    let background_ms: Vec<u128> = cold[..3]
        .iter()
        .map(|file| {
            let _ = spawn(visible, None);
            spawn(file, None).1
        })
        .collect();
    row("F", "background tab, newtab policy", &background_ms);

    let (tabs_after, frames_after) = views(&root);
    println!(
        "left on the desktop: {tabs_after} tab(s) in {frames_after} frame(s) \
         (start was {tabs_before}/{frames_before}); fixture-only frames are closed, \
         tabs merged into your own window are not"
    );
    powershell(CLOSE_FIXTURE_FRAMES, &root);
    // Explorer needs a moment to let go of the folders before they are deleted.
    std::thread::sleep(Duration::from_millis(700));
    let _ = std::fs::remove_dir_all(&root);
}
