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

use std::path::{Path, PathBuf};
use std::process::Command;

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

fn powershell(script: &str, dir: &Path, name: &str) -> String {
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env("REVEAL_UI_DIR", dir)
        .env("REVEAL_UI_NAME", name)
        .output()
        .expect("powershell should run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
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

#[test]
#[ignore = "opens a real Explorer window; run with --ignored and REVEAL_UI_TEST=1"]
fn real_explorer_reveal_selects_the_file() {
    if std::env::var_os("REVEAL_UI_TEST").is_none() {
        eprintln!("skipping real Explorer test; set REVEAL_UI_TEST=1 to opt in");
        return;
    }

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
