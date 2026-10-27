// Cross-check the hand-rolled shell bindings against the official `windows` crate.
//
// Compiled only for `cargo test`; the shipped binary still links nothing but system DLLs.
// The parsing and argument-marshalling paths run here; the one thing that still never runs is
// `SHOpenFolderAndSelectItems` with a *real* folder, because that opens an Explorer window. A
// null folder is allowed: measured on build 26100 it either fails (`E_INVALIDARG` with an empty
// array) or succeeds without doing anything visible (`S_OK` with one valid item), and neither
// shape creates or reactivates a window.
//
// What is being verified:
//   1. the verbatim-path failure that [`normalize`] works around is a property of the API,
//      not of our declarations: the official binding returns the same HRESULT;
//   2. the (parent, child) split that reveal feeds to `SHOpenFolderAndSelectItems` is exactly
//      what the official `IL*` helpers compute for the same path;
//   3. the empty-argument edges both bindings must agree on, because reveal's error paths reach
//      them (null folder, null PIDL);
//   4. the one-item array reveal actually passes, including — when the feature is on — the
//      slice reconstruction inside our own `ffi::official` wrapper.
//
// The hand-rolled side is reached as `ffi::native`, never through the flavour the build
// selected: under `--features official-bindings` the two sides would otherwise be the same
// binding and the comparison would prove nothing. One leg deliberately goes through the selected
// flavour instead, because covering the wrapper is the point there, and it is marked with the
// same `cfg(feature)` so it only exists where it means something.

use super::*;
use std::fs;
use std::os::raw::c_void;
use windows::core::HSTRING;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::{self, Common::ITEMIDLIST};

/// HRESULT of the official binding for `path`, with the PIDL it allocates released.
fn official_hr(path: &Path) -> u32 {
    let name = wide_name(path);
    unsafe {
        let mut pidl: *mut ITEMIDLIST = ptr::null_mut();
        let result = Shell::SHParseDisplayName(&name, None::<&IBindCtx>, &mut pidl, 0, None);
        let hr = match result {
            Ok(()) => 0,
            Err(e) => e.code().0,
        };
        Shell::ILFree(Some(pidl));
        hr as u32
    }
}

/// HRESULT of our own declaration for `path`, with the PIDL it allocates released.
fn our_hr(path: &Path) -> u32 {
    let wide = encode(path);
    unsafe {
        let mut pidl = ptr::null_mut();
        let hr = ffi::native::SHParseDisplayName(wide.as_ptr(), ptr::null(), &mut pidl, 0, ptr::null_mut());
        ffi::native::ILFree(pidl);
        hr as u32
    }
}

fn wide_name(path: &Path) -> HSTRING {
    let text: Vec<u16> = path.as_os_str().encode_wide().collect();
    HSTRING::from_wide(&text)
}

/// A crate `Result` down to the same `u32` HRESULT number the native flavour returns.
fn code_of(outcome: windows::core::Result<()>) -> u32 {
    match outcome {
        Ok(()) => 0,
        Err(error) => error.code().0 as u32,
    }
}

fn encode(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// The extended-length (`\\?\`) spelling of a local absolute path.
fn verbatim(path: &Path) -> PathBuf {
    PathBuf::from(OsString::from(format!(r"\\?\{}", path.display())))
}

/// `(cbSize, its bytes)` of an item id — the only part that identifies the item.
unsafe fn item_bytes(id: *const c_void) -> Vec<u8> {
    let size = *(id as *const u16) as usize;
    std::slice::from_raw_parts(id as *const u8, size).to_vec()
}

/// A COM apartment for the current test thread, released on unwind.
struct Com;

impl Com {
    fn init() -> Com {
        let hr = unsafe { ffi::native::CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED) };
        assert!(hr >= 0, "CoInitializeEx failed: 0x{hr:08X}");
        Com
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { ffi::native::CoUninitialize() };
    }
}

fn fixture(tag: &str, name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "reveal-xcheck-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    fs::write(&file, "x").unwrap();
    (dir, file)
}

#[test]
fn official_bindings_reject_verbatim_paths_exactly_like_ours() {
    let _com = Com::init();
    let (dir, file) = fixture("verbatim", r"空格 中文,file.txt");

    let extended = verbatim(&file);
    // The API itself refuses the extended spelling, through both declarations: `resolve`
    // rewriting it fixes a shell constraint rather than hiding a mistake of ours.
    assert_eq!(our_hr(&extended), 0x8007_0057, "our declaration changed the failure");
    assert_eq!(
        official_hr(&extended),
        0x8007_0057,
        "the official binding disagrees about the verbatim form"
    );
    // Both accept it once the prefix is gone, which is what resolve hands to the shell.
    let plain = normalize(&extended);
    assert_eq!(&*plain, file.as_path());
    assert_eq!(our_hr(&plain), 0, "{}", plain.display());
    assert_eq!(official_hr(&plain), 0, "official binding disagrees after the rewrite");

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn our_parent_child_split_matches_the_official_helpers() {
    let _com = Com::init();
    let (dir, file) = fixture("split", "target 文件.txt");

    unsafe {
        let name = wide_name(&file);
        let mut full: *mut ITEMIDLIST = ptr::null_mut();
        Shell::SHParseDisplayName(&name, None::<&IBindCtx>, &mut full, 0, None).unwrap();

        // Our own declarations, driven against an allocation the official parser produced, so
        // the only variable left is the code under test. (Two parses of one path are not
        // byte-identical: a folder's item id carries a timestamp the shell refreshes.)
        let ours = Shell::ILClone(full);
        let our_child = item_bytes(ffi::native::ILFindLastID(ours as *mut c_void));
        ffi::native::ILRemoveLastID(ours as *mut c_void);

        // ILIsParent(parent, child, immediate) — measured on this machine: the flag asks for
        // the direct parent when true and any ancestor when false, and a list is never its own
        // direct parent. So the first assert also proves our truncation removed exactly one
        // level, which is what reveal relies on when it opens `parent` and selects `child`.
        assert!(
            Shell::ILIsParent(ours, full, true).as_bool(),
            "our ILRemoveLastID did not leave the direct parent of the item"
        );
        assert!(
            Shell::ILIsParent(ours, full, false).as_bool(),
            "our truncated list is not even an ancestor of the item"
        );
        assert_eq!(
            our_child,
            item_bytes(Shell::ILFindLastID(full) as *const c_void),
            "our ILFindLastID does not return the official last id"
        );

        Shell::ILFree(Some(full));
        // Our ILFree on a block the official ILClone allocated: the two share one allocator.
        ffi::native::ILFree(ours as *mut c_void);
    }

    fs::remove_dir_all(&dir).unwrap();
}

/// The empty-argument path of the reveal call.
///
/// `SHOpenFolderAndSelectItems` is what the whole crate exists to call, yet until now no test
/// compared how the two bindings marshal its arguments — a real folder would pop Explorer. A
/// null folder is the safe exception: measured on build 26100 it returns `E_INVALIDARG` before
/// a window is created (`Shell.Windows` count unchanged across the call), so this pins the
/// four-parameter signature (`pidl_folder, cidl, apidl, dw_flags`) on both sides to the same
/// failure. Scope note: the official leg calls `Shell::` directly, which is what makes the two
/// sides genuinely different code — and also means this test says nothing about the array
/// reconstruction in our own `ffi::official` wrapper. That leg is the one-item array test below.
#[test]
fn both_bindings_refuse_a_null_folder_before_opening_one() {
    let _com = Com::init();
    let ours = unsafe { ffi::native::SHOpenFolderAndSelectItems(ptr::null(), 0, ptr::null(), 0) };
    let theirs = unsafe {
        Shell::SHOpenFolderAndSelectItems(ptr::null(), None::<&[*const ITEMIDLIST]>, 0u32)
    };
    assert_eq!(ours as u32, 0x8007_0057, "the null folder no longer fails the way it measured");
    assert_eq!(
        code_of(theirs),
        0x8007_0057,
        "the official binding marshals the null folder to a different failure"
    );
}

/// The PIDL every parse failure leaves behind.
///
/// When `SHParseDisplayName` fails our code still hands the (null) PIDL to `ILFree`, so a null
/// free is a live path, not a hypothetical: measured on build 26100 to be a safe no-op. Both
/// bindings must tolerate it, because they release the same shell-allocated blocks.
#[test]
fn both_bindings_free_a_null_pidl_without_crashing() {
    let _com = Com::init();
    unsafe { ffi::native::ILFree(ptr::null_mut()) };
    unsafe { Shell::ILFree(None) };
}

/// The one-item array reveal really passes, at the folder argument that cannot open a window.
///
/// `reveal_parsed` calls `SHOpenFolderAndSelectItems(parent, 1, &child, 0)`, so a non-empty array
/// is the only shape production uses — and the tests above pass either no array or an empty one,
/// which is exactly what skips the `Some(slice::from_raw_parts(apidl, cidl))` reconstruction in
/// the `ffi::official` wrapper. A null folder keeps it window-free: measured on build 26100 the
/// call returns
/// `S_OK` with `Shell.Windows` unchanged, and it is not a vacuous success — the shell dereferences
/// `apidl` *before* rejecting the folder, so a bogus pointer there took the probe process down with
/// an access violation. The item therefore has to come from a real parse, and a wrong base or
/// length in the reconstruction shows up as a crash or a different code rather than agreeing by
/// accident.
#[test]
fn both_bindings_marshal_a_one_item_array_to_the_same_result() {
    let _com = Com::init();
    let (dir, file) = fixture("one-item", "select 我.txt");

    unsafe {
        let wide = encode(&file);
        let mut full: *mut c_void = ptr::null_mut();
        let parsed = ffi::native::SHParseDisplayName(wide.as_ptr(), ptr::null(), &mut full, 0, ptr::null_mut());
        assert_eq!(parsed, 0, "the fixture stopped parsing: {}", file.display());
        let child = ffi::native::ILFindLastID(full);
        let child_ptr: *const c_void = child;

        let ours = ffi::native::SHOpenFolderAndSelectItems(ptr::null(), 1, &child_ptr, 0);
        let theirs = Shell::SHOpenFolderAndSelectItems(ptr::null(), Some(&[child as *const ITEMIDLIST]), 0u32);
        assert_eq!(ours as u32, 0, "a null folder with one valid item no longer returns S_OK");
        assert_eq!(
            code_of(theirs),
            0,
            "the official binding marshals the one-item array to a different result"
        );

        // The leg that only exists under the feature: this is our own wrapper, so the raw
        // pointer/count pair really is rebuilt into a slice before the shell sees it.
        #[cfg(feature = "official-bindings")]
        assert_eq!(
            ffi::SHOpenFolderAndSelectItems(ptr::null(), 1, &child_ptr, 0) as u32,
            0,
            "the official wrapper's slice reconstruction diverged from the raw call"
        );

        ffi::native::ILFree(full);
    }

    fs::remove_dir_all(&dir).unwrap();
}
