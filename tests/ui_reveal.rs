//! Opt-in test that really opens an Explorer window.
//!
//! Excluded from the default suite because it pops a window on the developer's desktop.
//! Run with:
//!
//! ```text
//! REVEAL_UI_TEST=1 cargo test --test ui_reveal -- --ignored --nocapture
//! ```
//!
//! It asserts both halves of the contract: the process exits 0, and the item is actually
//! highlighted. The highlight is read through UI Automation (`SelectionItemPattern.
//! IsSelected`), which is independent of the shell APIs the tool calls, so it verifies the
//! observed result rather than the request. Explorer creates the window asynchronously, so
//! the test polls. Afterwards it closes every Explorer window it opened and deletes the
//! fixture, because a window left behind by a failed run would pollute the next one.
//!
//! The two tests below share one desktop and one tabbed Explorer frame, so a run of one
//! moves the frame title the other is reading. They take turns on `UI_LOCK` instead of
//! asking the developer to remember `--test-threads=1`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// Serializes the real-window tests: they read each other's state through the shell.
static UI_LOCK: Mutex<()> = Mutex::new(());

/// Takes the desktop-wide lock, for as long as the test needs to move tabs around.
///
/// A poisoned lock still grants the lock: whoever panicked left the shell in some state,
/// and that is not a reason to make the other test report a lock failure instead of running.
fn lock_desktop() -> std::sync::MutexGuard<'static, ()> {
    UI_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// UI Automation query: prints "<found> <selected> <renaming>" for the ListItems named
/// `$env:REVEAL_UI_NAME`.
///
/// `renaming` counts Edit controls titled with the item's own name: an open in-place
/// rename editor is an Edit named after the file. Edit controls elsewhere (the details
/// pane's 类型/修改日期/属性/大小 fields) are named after the property, so they do not
/// count.
const SELECTION_QUERY: &str = "\
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes; \
$root=[System.Windows.Automation.AutomationElement]::RootElement; \
$li=New-Object System.Windows.Automation.PropertyCondition(\
[System.Windows.Automation.AutomationElement]::ControlTypeProperty,\
[System.Windows.Automation.ControlType]::ListItem); \
$edit=New-Object System.Windows.Automation.PropertyCondition(\
[System.Windows.Automation.AutomationElement]::ControlTypeProperty,\
[System.Windows.Automation.ControlType]::Edit); \
$found=0; $selected=0; \
$edits=$root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$edit); \
foreach($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$li)){ \
  if($e.Current.Name -eq $env:REVEAL_UI_NAME){ \
    $found++; \
    if($e.GetCurrentPropertyValue([System.Windows.Automation.SelectionItemPattern]::IsSelectedProperty)){ $selected++ } \
  } \
}; \
$renaming=0; \
foreach($e in $edits){ if($e.Current.Name -eq $env:REVEAL_UI_NAME){ $renaming++ } }; \
\"$found $selected $renaming\"";

/// Closes the Explorer windows that show *only* `$env:REVEAL_UI_DIR`.
///
/// `IWebBrowser2.Quit` is ignored by Explorer on current Windows 11 builds, so the
/// window is asked to close the way the title-bar button does: `WM_CLOSE` to its HWND.
/// A window that hosts the fixture as one tab among others is left alone, because its
/// frame HWND is shared with the user's own tabs and `WM_CLOSE` would close all of them.
const CLOSE_WINDOWS: &str = "\
Add-Type -Name NativeWindow -Namespace Reveal -MemberDefinition \
'[DllImport(\"user32.dll\")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);'; \
$shell=New-Object -ComObject Shell.Application; \
$windows=$shell.Windows(); \
$frames=@{}; \
for($i=0; $i -lt $windows.Count; $i++){ \
  $w=$windows.Item($i); \
  try{ \
    $h=[IntPtr]$w.HWND; \
    $p=$w.Document.Folder.Self.Path; \
    if(-not $frames.ContainsKey($h)){ $frames[$h]=@() }; \
    $frames[$h]+=$p \
  }catch{ } \
}; \
foreach($h in $frames.Keys){ \
  $only=$true; \
  foreach($p in $frames[$h]){ if($p -ne $env:REVEAL_UI_DIR){ $only=$false } }; \
  if($only){ [Reveal.NativeWindow]::PostMessage($h,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null } \
}";

/// How many `Shell.Windows` entries (one per tab) show `$env:REVEAL_UI_DIR`.
const COUNT_VIEWS: &str = "\
$shell=New-Object -ComObject Shell.Application; \
$n=0; \
foreach($w in $shell.Windows()){ \
  try{ if($w.Document.Folder.Self.Path -eq $env:REVEAL_UI_DIR){ $n++ } }catch{ } \
}; \
\"$n\"";

/// `<tabs showing REVEAL_UI_DIR> <tabs showing REVEAL_UI_NAME> <all tabs>`, in one process.
///
/// The three numbers go in a single script because the settle poll below reads them
/// repeatedly, and each PowerShell start-up costs more than the query it runs. The total is
/// the number `overwrite` has to keep constant: it repurposes the tab the user is looking at
/// instead of adding or switching to one.
const TAB_TRIPLE: &str = "\
$shell=New-Object -ComObject Shell.Application; \
$a=0; $b=0; $n=$shell.Windows().Count; \
foreach($w in $shell.Windows()){ \
  try{ $p=$w.Document.Folder.Self.Path; \
    if($p -eq $env:REVEAL_UI_DIR){ $a++ } elseif($p -eq $env:REVEAL_UI_NAME){ $b++ } \
  }catch{ } \
}; \
\"$a $b $n\"";

/// Minimizes the frame hosting `$env:REVEAL_UI_DIR`, and prints how many of those frames
/// are minimized afterwards.
///
/// One script does both because the count is meaningless until the minimize has happened,
/// and Explorer takes its own time to act on `ShowWindow`. Both `DllImport`s have to sit in
/// one `-MemberDefinition` string: PowerShell does not evaluate `'a' + 'b'` in command
/// position, and a failed `Add-Type` would leave the count at a quiet, misleading `0`.
///
/// With a tabbed shell the fixture usually lives in a tab of the user's own window, so this
/// minimizes that window; the reveal under test is what has to bring it back.
const MINIMIZE_AND_COUNT_ICONIC: &str = "\
Add-Type -Name Iconic -Namespace Reveal -MemberDefinition \
'[DllImport(\"user32.dll\")] public static extern int ShowWindow(IntPtr h, int c); \
[DllImport(\"user32.dll\")] public static extern int IsIconic(IntPtr h);'; \
$shell=New-Object -ComObject Shell.Application; \
$frames=@{}; \
foreach($w in $shell.Windows()){ \
  try{ \
    if($w.Document.Folder.Self.Path -eq $env:REVEAL_UI_DIR){ \
      $frames[[IntPtr]$w.HWND]=0; \
      [Reveal.Iconic]::ShowWindow([IntPtr]$w.HWND,6)|Out-Null \
    } \
  }catch{ } \
}; \
Start-Sleep -Milliseconds 500; \
$n=0; \
foreach($h in $frames.Keys){ if([Reveal.Iconic]::IsIconic($h) -ne 0){ $n++ } }; \
\"$n\"";

/// How many frames showing `$env:REVEAL_UI_DIR` are still minimized.
const COUNT_ICONIC: &str = "\
Add-Type -Name StillIconic -Namespace Reveal -MemberDefinition \
'[DllImport(\"user32.dll\")] public static extern int IsIconic(IntPtr h);'; \
$shell=New-Object -ComObject Shell.Application; \
$frames=@{}; \
foreach($w in $shell.Windows()){ \
  try{ \
    if($w.Document.Folder.Self.Path -eq $env:REVEAL_UI_DIR){ $frames[[IntPtr]$w.HWND]=0 } \
  }catch{ } \
}; \
$n=0; \
foreach($h in $frames.Keys){ if([Reveal.StillIconic]::IsIconic($h) -ne 0){ $n++ } }; \
\"$n\"";

/// Titles of the Explorer frames, one per line.
///
/// A tabbed window puts the *visible* tab's name at the front of its title bar, so a title
/// starting with a fixture folder's leaf name is the only available proof that this folder is
/// what the user is actually looking at. `Shell.Windows` reports one entry per tab with no
/// field for the active one, and the fixture leaf names carry the test's process id so a tab
/// left behind by an older run cannot answer for the current one.
const FRAME_TITLES: &str = "\
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes; \
$root=[System.Windows.Automation.AutomationElement]::RootElement; \
$cw=New-Object System.Windows.Automation.PropertyCondition(\
[System.Windows.Automation.AutomationElement]::ClassNameProperty,\
'CabinetWClass'); \
$lines=@(); \
foreach($e in $root.FindAll([System.Windows.Automation.TreeScope]::Children,$cw)){ \
  $lines += $e.Current.Name \
}; \
$lines -join \"`n\"";

fn powershell_env(script: &str, vars: &[(&str, &OsStr)]) -> String {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-Command", script]);
    for (key, value) in vars {
        cmd.env(key, value);
    }
    let out = cmd.output().expect("powershell should run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn powershell(script: &str, dir: &Path, name: &str) -> String {
    powershell_env(
        script,
        &[
            ("REVEAL_UI_DIR", dir.as_os_str()),
            ("REVEAL_UI_NAME", OsStr::new(name)),
        ],
    )
}

/// `(found, selected, renaming)` counts for the fixture item.
fn selection(dir: &Path, name: &str) -> (i64, i64, i64) {
    let report = powershell(SELECTION_QUERY, dir, name);
    let mut parts = report.split_whitespace().map(|v| v.parse().unwrap_or(-1));
    (
        parts.next().unwrap_or(-1),
        parts.next().unwrap_or(-1),
        parts.next().unwrap_or(-1),
    )
}

/// How many Explorer frames showing `dir` are minimized.
fn iconic_count(dir: &Path, script: &str) -> i64 {
    powershell(script, dir, "")
        .trim()
        .parse()
        .unwrap_or(-1)
}

/// How many tabs show `a`, how many show `b`, and how many tabs exist in total.
fn tab_counts(a: &Path, b: &Path) -> (i64, i64, i64) {
    let vars = [
        ("REVEAL_UI_DIR", a.as_os_str()),
        ("REVEAL_UI_NAME", b.as_os_str()),
    ];
    let report = powershell_env(TAB_TRIPLE, &vars);
    let mut parts = report.split_whitespace().map(|v| v.parse().unwrap_or(-1));
    (
        parts.next().unwrap_or(-1),
        parts.next().unwrap_or(-1),
        parts.next().unwrap_or(-1),
    )
}

/// Tab counts read twice in a row that agree.
///
/// `Navigate2` is asynchronous, so the shell is still moving tabs after the tool has already
/// returned 0. Waiting for two identical readings beats encoding what we expect to settle on —
/// the expectation is the assertion, not the poll.
fn settled_tab_counts(a: &Path, b: &Path) -> (i64, i64, i64) {
    let mut previous = tab_counts(a, b);
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let now = tab_counts(a, b);
        if now == previous {
            return now;
        }
        previous = now;
    }
    previous
}

/// The title bar of every Explorer frame on the desktop.
fn frame_titles() -> Vec<String> {
    powershell_env(FRAME_TITLES, &[])
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Whether some Explorer frame has `dir` as the tab the user is looking at.
///
/// The title bar is the only visible-tab signal the tabbed shell gives: `Shell.Windows`
/// lists one entry per tab with nothing to say which one is on screen. A frame names its
/// visible tab first, then appends the rest (` 和 N 个其他选项卡 …`, or ` - 文件资源管理器`
/// for a one-tab window), and with "display full path" switched on it names it by path
/// instead of by folder name — so both spellings are tried, case-insensitively, and the
/// character after the match has to be a space. Matching a loose substring instead would
/// call `over-a-1` on screen while the user is reading `over-a-12`.
fn visible_in_title(titles: &[String], dir: &Path) -> bool {
    let leaf = dir.file_name().unwrap_or_default().to_string_lossy();
    let full = dir.as_os_str().to_string_lossy();
    titles.iter().any(|title| {
        let title = title.to_lowercase();
        [leaf.as_ref(), full.as_ref()].into_iter().any(|head| {
            let head = head.to_lowercase();
            !head.is_empty()
                && title.starts_with(&head)
                && title.get(head.len()..).is_none_or(|rest| rest.starts_with(' '))
        })
    })
}

/// Polls UI Automation until the item shows as selected, or gives up.
fn selected(dir: &Path, name: &str) -> (i64, i64, i64) {
    let mut last = (0, 0, 0);
    for _ in 0..20 {
        last = selection(dir, name);
        if last.1 > 0 {
            return last;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    last
}

/// Runs the binary for real, stating both settings instead of inheriting them.
fn reveal(path: &Path, policy: &str) -> i32 {
    Command::new(env!("CARGO_BIN_EXE_reveal"))
        .arg(path)
        .env_remove("REVEAL_DRY_RUN")
        .env("REVEAL_TAB_POLICY", policy)
        .output()
        .expect("reveal binary should run")
        .status
        .code()
        .unwrap_or(-1)
}

#[test]
#[ignore = "opens a real Explorer window; run with --ignored and REVEAL_UI_TEST=1"]
fn real_explorer_reveal_selects_the_file() {
    if std::env::var_os("REVEAL_UI_TEST").is_none() {
        eprintln!("skipping real Explorer test; set REVEAL_UI_TEST=1 to opt in");
        return;
    }
    let _desktop = lock_desktop();

    let dir = std::env::temp_dir().join(format!(
        "reveal-ui-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let name = "ZZ-TARGET file.txt";
    let file: PathBuf = dir.join(name);
    std::fs::write(&file, "x").unwrap();

    let result = std::panic::catch_unwind(|| {
        let out = Command::new(env!("CARGO_BIN_EXE_reveal"))
            .arg(&file)
            .env_remove("REVEAL_DRY_RUN")
            .output()
            .expect("reveal binary should run");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");

        let mut last = (0, 0, 0);
        for _ in 0..20 {
            last = selection(&dir, name);
            if last.1 > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        eprintln!(
            "{}: found {} UIA ListItem(s), {} selected, {} in rename editor",
            file.display(),
            last.0,
            last.1,
            last.2
        );
        assert!(last.0 > 0, "the file never appeared in any Explorer view");
        assert!(
            last.1 > 0,
            "the file is visible but not highlighted (found {}, selected {})",
            last.0,
            last.1
        );
        // Revealing is not renaming: SVSI_EDIT (0x3) would drop the item into the
        // in-place rename editor, which UIA shows as an Edit child of the list item.
        assert_eq!(last.2, 0, "the revealed item opened in rename mode, not selected");

        // Revealing a second file in the same folder must reuse the open window instead
        // of spawning another one (the shell otherwise opens a new window whenever the
        // folder is only reachable through a background tab).
        let views_before = powershell(COUNT_VIEWS, &dir, name);
        // Guard the comparison itself: a script that silently finds nothing would report 0
        // views and make "no window added" true for the wrong reason.
        assert!(
            views_before.trim().parse::<i64>().unwrap_or(0) >= 1,
            "no Explorer view reports the fixture folder, so reuse cannot be checked \
             (count was {views_before})"
        );
        let name2 = "ZZ-TARGET2 file.txt";
        let file2: PathBuf = dir.join(name2);
        std::fs::write(&file2, "y").unwrap();
        let out2 = Command::new(env!("CARGO_BIN_EXE_reveal"))
            .arg(&file2)
            .env_remove("REVEAL_DRY_RUN")
            .output()
            .expect("reveal binary should run");
        assert_eq!(out2.status.code(), Some(0), "second reveal failed");
        let views_after = powershell(COUNT_VIEWS, &dir, name2);
        assert_eq!(
            views_before, views_after,
            "reveal opened another window for an already-open folder"
        );
        let mut last2 = (0, 0, 0);
        for _ in 0..20 {
            last2 = selection(&dir, name2);
            if last2.1 > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        assert!(
            last2.1 > 0,
            "second file not highlighted in the reused window (found {}, selected {})",
            last2.0,
            last2.1
        );
        assert_eq!(last2.2, 0, "the second reveal opened a rename editor");

        // A minimized window is out of sight in a way the title cannot express: its tab is
        // still "the visible tab", so selecting succeeds while the user sees nothing. The
        // reveal has to undo the minimization itself.
        let minimized = iconic_count(&dir, MINIMIZE_AND_COUNT_ICONIC);
        assert_eq!(minimized, 1, "the fixture window did not minimize; nothing to restore");
        let name3 = "ZZ-TARGET3 file.txt";
        let file3: PathBuf = dir.join(name3);
        std::fs::write(&file3, "z").unwrap();
        let out3 = Command::new(env!("CARGO_BIN_EXE_reveal"))
            .arg(&file3)
            .env_remove("REVEAL_DRY_RUN")
            .output()
            .expect("reveal binary should run");
        assert_eq!(out3.status.code(), Some(0), "third reveal failed");
        let mut last3 = (0, 0, 0);
        for _ in 0..20 {
            if iconic_count(&dir, COUNT_ICONIC) == 0 {
                last3 = selection(&dir, name3);
                if last3.1 > 0 {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        assert_eq!(
            iconic_count(&dir, COUNT_ICONIC),
            0,
            "reveal left the window minimized, where the user cannot see the selection"
        );
        assert!(last3.1 > 0, "third file not highlighted after the window was restored");
        assert_eq!(last3.2, 0, "the third reveal opened a rename editor");
    });

    powershell(CLOSE_WINDOWS, &dir, name);
    // Give Explorer a moment to release the folder before the fixture is deleted.
    std::thread::sleep(std::time::Duration::from_millis(700));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_ok(),
        "fixture {} was cleaned up after the failure above",
        dir.display()
    );
}

/// `REVEAL_TAB_POLICY=overwrite` takes over the tab the user is reading instead of adding
/// one, which is the half of the tab story that `real_explorer_reveal_selects_the_file`
/// cannot see: with the default policy the visible tab is never touched.
///
/// Two fixture folders are needed to make the trade-off observable — one parked in a
/// background tab (the target) and one on screen (the one that gets spent). The promises
/// under test, all read back from the shell rather than from what the tool asked for:
/// the item ends up selected, the title bar then names the target's tab, the total number
/// of tabs does not move, the target gains a tab, and the folder the user was reading
/// loses one.
#[test]
#[ignore = "moves real Explorer tabs; run with --ignored and REVEAL_UI_TEST=1"]
fn overwrite_policy_repurposes_the_visible_tab_without_adding_one() {
    if std::env::var_os("REVEAL_UI_TEST").is_none() {
        eprintln!("skipping real Explorer test; set REVEAL_UI_TEST=1 to opt in");
        return;
    }
    let _desktop = lock_desktop();

    // The leaf names are mutually non-prefix and carry this process's id, so a tab left
    // behind by an older run cannot answer for the folders this run is watching.
    let pid = std::process::id();
    let target = std::env::temp_dir().join(format!("over-a-{pid}"));
    let foreground = std::env::temp_dir().join(format!("over-b-{pid}"));
    let (first, second, other) = ("ZZ-A1.txt", "ZZ-A2.txt", "ZZ-B1.txt");
    let target_files: &[&str] = &[first, second];
    let foreground_files: &[&str] = &[other];
    for (dir, names) in [
        (target.as_path(), target_files),
        (foreground.as_path(), foreground_files),
    ] {
        std::fs::create_dir_all(dir).unwrap();
        for name in names {
            std::fs::write(dir.join(name), "x").unwrap();
        }
    }
    let target_file: PathBuf = target.join(second);

    let result = std::panic::catch_unwind(|| {
        // Lay the situation out with the default policy: the target folder opens first and
        // stays open, then the foreground folder is revealed on top of it. Whichever way the
        // shell splits its windows, both folders end up with a tab and the second one in view.
        let setup1: PathBuf = target.join(first);
        let setup2: PathBuf = foreground.join(other);
        assert_eq!(reveal(&setup1, "newtab"), 0, "setup: target folder did not open");
        assert_eq!(reveal(&setup2, "newtab"), 0, "setup: foreground folder did not open");

        let (target_before, fore_before, total_before) = settled_tab_counts(&target, &foreground);
        let titles = frame_titles();
        eprintln!(
            "before: {target_before} target tab(s), {fore_before} foreground tab(s), \
             {total_before} tabs in total; titles {titles:?}"
        );
        assert!(
            target_before >= 1,
            "the target folder is not open in any tab, so there is no background tab to \
             repurpose ({target_before})"
        );
        assert!(
            visible_in_title(&titles, &foreground) && !visible_in_title(&titles, &target),
            "the shell put each fixture in a window of its own, so the target is already on \
             screen and `overwrite` has nothing to take over \
             (foreground visible: {}, target visible: {}, titles {titles:?})",
            visible_in_title(&titles, &foreground),
            visible_in_title(&titles, &target)
        );

        assert_eq!(
            reveal(&target_file, "overwrite"),
            0,
            "overwrite reveal of {} failed",
            target_file.display()
        );

        // Selection, read from UI Automation. What this proves on its own is only that a tab
        // holding the folder highlighted the file: measured on build 26100, a background tab
        // keeps its item list in the UIA tree, so two tabs of the same folder both report
        // `IsSelected` while the user is looking at neither. That is what the title-bar
        // assertions below are for — together the two halves say "highlighted, and on screen".
        let last = selected(&target, second);
        eprintln!(
            "{}: found {} UIA ListItem(s), {} selected, {} in rename editor",
            target_file.display(),
            last.0,
            last.1,
            last.2
        );
        assert!(last.0 > 0, "the target folder was never drawn with the file in it");
        assert!(last.1 > 0, "the file is drawn but not highlighted (selected {})", last.1);
        assert_eq!(last.2, 0, "the overwrite reveal opened a rename editor");

        let (target_after, fore_after, total_after) = settled_tab_counts(&target, &foreground);
        let titles = frame_titles();
        eprintln!(
            "after: {target_after} target tab(s), {fore_after} foreground tab(s), \
             {total_after} tabs in total; titles {titles:?}"
        );
        assert_eq!(
            total_after, total_before,
            "overwrite added a tab ({total_before} -> {total_after}); it was supposed to \
             spend the tab the user was reading instead"
        );
        assert_eq!(
            target_after,
            target_before + 1,
            "no tab was repurposed for the target folder ({target_before} -> {target_after})"
        );
        assert_eq!(
            fore_after + 1,
            fore_before,
            "the foreground folder lost a tab somewhere other than the one overwrite took \
             ({fore_before} -> {fore_after})"
        );
        assert!(
            visible_in_title(&titles, &target),
            "the target folder is open but still not the tab on screen; titles {titles:?}"
        );
        assert!(
            !visible_in_title(&titles, &foreground),
            "the tab the user was reading still shows the foreground folder; titles {titles:?}"
        );
    });

    // Both fixtures are closed only where a frame holds nothing else, so a frame shared
    // with the user's own tabs keeps its tabs; leftover tabs are the cost of this test and
    // are the user's to `Ctrl+W`.
    powershell(CLOSE_WINDOWS, &target, second);
    powershell(CLOSE_WINDOWS, &foreground, other);
    std::thread::sleep(std::time::Duration::from_millis(700));
    let _ = std::fs::remove_dir_all(&target);
    let _ = std::fs::remove_dir_all(&foreground);
    assert!(
        result.is_ok(),
        "fixtures {} and {} were cleaned up after the failure above",
        target.display(),
        foreground.display()
    );
}
