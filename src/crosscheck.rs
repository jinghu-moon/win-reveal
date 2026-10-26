// Cross-check the hand-rolled shell bindings against the official `windows` crate.
//
// Compiled only for `cargo test`; the shipped binary still links nothing but system DLLs.
// Only namespace *parsing* runs here — never `SHOpenFolderAndSelectItems` — so no Explorer
// window is opened and these tests belong to the default suite.
//
// What is being verified:
//   1. the verbatim-path failure that [`normalize`] works around is a property of the API,
//      not of our declarations: the official binding returns the same HRESULT;
//   2. the (parent, child) split that reveal feeds to `SHOpenFolderAndSelectItems` is exactly
//      what the official `IL*` helpers compute for the same path.
//
// The hand-rolled side is reached as `ffi::native`, never through the flavour the build
// selected: under `--features official-bindings` the two sides would otherwise be the same
// binding and the comparison would prove nothing.

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
