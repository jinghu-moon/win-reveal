//! Minimal late-bound IDispatch automation client, hand-rolled to keep the crate
//! dependency-free.
//!
//! It exists for exactly one purpose: some Windows 11 shells (observed on build 26100)
//! open the folder but silently drop the selection requested by
//! `SHOpenFolderAndSelectItems`. The folder view of a running Explorer window can still
//! be told to select an item through `Shell.Application` -> window -> `Document`
//! (the folder view) -> `SelectItem`, which is what this module drives.
//!
//! Everything is resolved by name through `GetIDsOfNames`/`Invoke`, so no interface
//! vtable layout beyond `IDispatch` itself is assumed.

use std::ffi::{OsStr, OsString};
use std::os::raw::c_void;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::ptr;

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const IID_IDISPATCH: Guid = Guid {
    d1: 0x0002_0400,
    d2: 0,
    d3: 0,
    d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
};

/// `GetIDsOfNames`/`Invoke` document their `riid` as reserved; it must be null.
const IID_NULL: Guid = Guid {
    d1: 0,
    d2: 0,
    d3: 0,
    d4: [0; 8],
};

const LOCALE_USER_DEFAULT: u32 = 0x0400;
const DISPATCH_METHOD: u16 = 1;
const DISPATCH_PROPERTYGET: u16 = 2;
/// Members are resolved by name only, so accept either shape: `Windows` and
/// `SelectItem` are methods, `Count`/`Path`/`Folder` are properties.
const INVOKE_FLAGS: u16 = DISPATCH_METHOD | DISPATCH_PROPERTYGET;
/// `SelectItem` state flags, from `_SVSIF` in ShObjIdl_core.idl:
/// `SVSI_SELECT` 0x1, `SVSI_DESELECTOTHERS` 0x4, `SVSI_ENSUREVISIBLE` 0x8,
/// `SVSI_FOCUSED` 0x10.
///
/// The trap is `SVSI_EDIT` (0x3), which *includes* `SVSI_SELECT`: setting bits 0 and 1
/// together — plainly, or inside the older 0x7 — asks the view to open the in-place
/// rename editor, and the observed symptom was the revealed file entering rename mode.
const SVSI_SELECT_AND_SHOW: i32 = 0x1 | 0x4 | 0x8 | 0x10;

const VT_EMPTY: u16 = 0;
const VT_I4: u16 = 3;
const VT_BSTR: u16 = 8;
const VT_DISPATCH: u16 = 9;
const VT_BYREF: u16 = 0x4000;

#[repr(C)]
struct Variant {
    vt: u16,
    reserved: [u16; 3],
    a: u64,
    b: u64,
}

#[repr(C)]
struct DispParams {
    args: *mut Variant,
    named: *mut i32,
    c_args: u32,
    c_named: u32,
}

#[repr(C)]
struct Vtable {
    query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_type_info_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    get_type_info: unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32,
    get_ids_of_names: unsafe extern "system" fn(
        *mut c_void,
        *const Guid,
        *const *const u16,
        u32,
        u32,
        *mut i32,
    ) -> i32,
    invoke: unsafe extern "system" fn(
        *mut c_void,
        i32,
        *const Guid,
        u32,
        u16,
        *mut DispParams,
        *mut Variant,
        *mut c_void,
        *mut u32,
    ) -> i32,
}

mod ole {
    use super::{Guid, Variant};
    use std::os::raw::c_void;

    #[link(name = "ole32")]
    extern "system" {
        pub fn CLSIDFromProgID(prog_id: *const u16, clsid: *mut Guid) -> i32;
        pub fn CoCreateInstance(
            clsid: *const Guid,
            outer: *mut c_void,
            cls_context: u32,
            iid: *const Guid,
            out: *mut *mut c_void,
        ) -> i32;
    }

    #[link(name = "oleaut32")]
    extern "system" {
        pub fn VariantClear(variant: *mut Variant) -> i32;
        pub fn SysAllocString(text: *const u16) -> *mut u16;
    }
}

impl Variant {
    fn empty() -> Self {
        Variant {
            vt: VT_EMPTY,
            reserved: [0; 3],
            a: 0,
            b: 0,
        }
    }

    fn i4(value: i32) -> Self {
        Variant {
            vt: VT_I4,
            reserved: [0; 3],
            a: value as u32 as u64,
            b: 0,
        }
    }

    fn dispatch(pointer: *mut c_void) -> Self {
        Variant {
            vt: VT_DISPATCH,
            reserved: [0; 3],
            a: pointer as u64,
            b: 0,
        }
    }

    /// A BSTR variant that owns its string; `VariantClear` frees it. Argument variants
    /// must own their payload for exactly that reason: a borrowed pointer handed to
    /// `VariantClear` on drop corrupts the heap (observed as `0xC0000374` in-process).
    fn bstr(text: &[u16]) -> Self {
        let mut variant = Variant::empty();
        variant.vt = VT_BSTR;
        variant.a = unsafe { ole::SysAllocString(text.as_ptr()) } as u64;
        variant
    }

    /// Takes ownership of a contained IDispatch, leaving the variant empty.
    /// A VT_DISPATCH variant may legally carry a null pointer, meaning "no object" —
    /// e.g. `Document` on a shell window that hosts no folder view.
    fn take_dispatch(&mut self) -> Option<Dispatch> {
        if self.vt != VT_DISPATCH || self.a == 0 {
            return None;
        }
        let pointer = self.a as *mut c_void;
        self.vt = VT_EMPTY;
        Some(Dispatch(pointer))
    }

    /// Reads and frees a contained BSTR.
    fn take_string(&mut self) -> Option<OsString> {
        if (self.vt != VT_BSTR && self.vt != VT_BYREF | VT_BSTR) || self.a == 0 {
            return None;
        }
        let bstr = self.a as *const u16;
        let mut len = 0usize;
        while unsafe { *bstr.add(len) } != 0 {
            len += 1;
        }
        let text = OsString::from_wide(unsafe { std::slice::from_raw_parts(bstr, len) });
        unsafe { ole::VariantClear(self) };
        Some(text)
    }

    fn as_i4(&self) -> Option<i64> {
        if self.vt == VT_I4 {
            Some(self.a as u32 as i32 as i64)
        } else {
            None
        }
    }

    /// The raw payload of a pointer-sized variant. `IWebBrowser2.HWND` is documented as
    /// "a pointer-sized integer coded as VT_DISPATCH", but this shell hands it back under
    /// a non-standard type code with the handle in the payload, so anything non-zero is
    /// tested with `IsWindow` (validated by the kernel) before being trusted.
    fn as_pointer(&self) -> Option<i64> {
        match self.vt {
            VT_DISPATCH | VT_I4 if self.a != 0 => Some(self.a as i64),
            _ if self.a != 0 => {
                let candidate = self.a as isize;
                (unsafe { IsWindow(candidate) } != 0).then_some(candidate as i64)
            }
            _ => None,
        }
    }
}

impl Drop for Variant {
    fn drop(&mut self) {
        unsafe { ole::VariantClear(self) };
    }
}

/// Owned IDispatch pointer.
pub struct Dispatch(*mut c_void);

impl Dispatch {
    fn vtable(&self) -> &Vtable {
        unsafe { &**(self.0 as *const *const Vtable) }
    }

    fn add_ref(&self) {
        unsafe { (self.vtable().add_ref)(self.0) };
    }

    fn invoke(&self, name: &str, args: &[Variant]) -> Result<Variant, String> {
        unsafe {
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let name_ptr: *const u16 = wide.as_ptr();
            let mut dispid: i32 = 0;
            let hr = (self.vtable().get_ids_of_names)(
                self.0,
                &IID_NULL,
                &name_ptr,
                1,
                LOCALE_USER_DEFAULT,
                &mut dispid,
            );
            if hr < 0 {
                return Err(format!("{name}: GetIDsOfNames HRESULT 0x{hr:08X}"));
            }
            // DISPPARAMS expects arguments in reverse order.
            let mut reversed: Vec<Variant> = args.iter().map(clone_variant).rev().collect();
            let mut params = DispParams {
                args: reversed.as_mut_ptr(),
                named: ptr::null_mut(),
                c_args: reversed.len() as u32,
                c_named: 0,
            };
            let mut result = Variant::empty();
            let hr = (self.vtable().invoke)(
                self.0,
                dispid,
                &IID_NULL,
                LOCALE_USER_DEFAULT,
                INVOKE_FLAGS,
                &mut params,
                &mut result,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if hr < 0 {
                return Err(format!("{name}: Invoke HRESULT 0x{hr:08X}"));
            }
            Ok(result)
        }
    }

    fn get_dispatch(&self, property: &str) -> Result<Dispatch, String> {
        let mut variant = self.invoke(property, &[])?;
        variant
            .take_dispatch()
            .ok_or_else(|| format!("{property}: not an object"))
    }

    fn get_string(&self, property: &str) -> Result<OsString, String> {
        let mut variant = self.invoke(property, &[])?;
        variant
            .take_string()
            .ok_or_else(|| format!("{property}: not a string"))
    }

    fn get_i4(&self, property: &str) -> Result<i64, String> {
        self.invoke(property, &[])?
            .as_i4()
            .ok_or_else(|| format!("{property}: not an integer"))
    }

    /// `IWebBrowser2.HWND` — the tab host frame shared by every tab of one window.
    fn hwnd(&self) -> Result<i64, String> {
        self.invoke("HWND", &[])?
            .as_pointer()
            .ok_or_else(|| "HWND: not a window handle".to_string())
    }

    fn item(&self, index: i64) -> Result<Dispatch, String> {
        let mut variant = self.invoke("Item", &[Variant::i4(index as i32)])?;
        variant
            .take_dispatch()
            .ok_or_else(|| format!("Item({index}): not an object"))
    }
}

impl Drop for Dispatch {
    fn drop(&mut self) {
        unsafe { (self.vtable().release)(self.0) };
    }
}

fn clone_variant(variant: &Variant) -> Variant {
    // Only the three argument types this module ever passes need cloning.
    match variant.vt {
        VT_I4 => Variant::i4(variant.a as u32 as i32),
        VT_BSTR => Variant::bstr(unsafe { wstr(variant.a as *const u16) }),
        VT_DISPATCH => {
            let borrowed = Dispatch(variant.a as *mut c_void);
            borrowed.add_ref();
            std::mem::forget(borrowed);
            Variant::dispatch(variant.a as *mut c_void)
        }
        _ => Variant::empty(),
    }
}

/// Borrows a nul-terminated UTF-16 string (a BSTR's payload) without reading its header.
unsafe fn wstr<'a>(text: *const u16) -> &'a [u16] {
    let mut len = 0usize;
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    unsafe { std::slice::from_raw_parts(text, len) }
}

fn normalized(path: &OsStr) -> Vec<u16> {
    let mut wide: Vec<u16> = path.encode_wide().collect();
    // Explorer reports `file:///C:/...` style locations while the caller may pass either
    // separator; treat both as the same directory.
    for c in wide.iter_mut() {
        if *c == b'/' as u16 {
            *c = b'\\' as u16;
        }
    }
    while let Some(&last) = wide.last() {
        if last == b'\\' as u16 || last == b'/' as u16 {
            wide.pop();
        } else {
            break;
        }
    }
    wide.into_iter()
        .map(|c| {
            if (b'A' as u16..=b'Z' as u16).contains(&c) {
                c + 32
            } else {
                c
            }
        })
        .collect()
}

fn same_path(a: &OsStr, b: &OsStr) -> bool {
    normalized(a) == normalized(b)
}

/// Why no selection could be applied.
pub enum Miss {
    /// No Explorer tab shows the parent folder.
    NoView,
    /// The folder is open, but only in a tab the user is not looking at, and the tab policy
    /// refuses to take over the visible tab. The caller can still ask the shell to activate
    /// that tab; see `activate_tab_by_path`.
    Hidden,
    /// A visible tab shows the parent folder but does not list the item yet — Explorer
    /// refreshes its item list asynchronously, so a just-created file can be missing for a
    /// moment.
    ItemNotListed,
}

/// `Shell.Application` -> `Windows`.
fn shell_windows() -> Result<Dispatch, Miss> {
    unsafe {
        let prog: Vec<u16> = "Shell.Application".encode_utf16().chain([0]).collect();
        let mut clsid = Guid {
            d1: 0,
            d2: 0,
            d3: 0,
            d4: [0; 8],
        };
        let hr = ole::CLSIDFromProgID(prog.as_ptr(), &mut clsid);
        if hr < 0 {
            return Err(Miss::NoView);
        }
        let mut raw = ptr::null_mut();
        let hr = ole::CoCreateInstance(&clsid, ptr::null_mut(), 0x1 | 0x4, &IID_IDISPATCH, &mut raw);
        if hr < 0 {
            return Err(Miss::NoView);
        }
        let shell = Dispatch(raw);
        shell.get_dispatch("Windows").map_err(|_| Miss::NoView)
    }
}

/// One `Shell.Windows` entry: its window handle, folder path and display name, the tab's
/// own browser (`window`), and the folder view (`document`) that can select items in it.
struct View {
    hwnd: i64,
    path: OsString,
    /// What the tab is called in the title bar, which is not always the leaf of `path`
    /// (`C:\Users\x\Documents` displays as 文档 on a Chinese shell).
    display: OsString,
    window: Dispatch,
    document: Dispatch,
    folder: Dispatch,
}

/// `Shell.Windows` exposes one entry per tab, so a single HWND appears repeatedly.
/// No entry says whether its tab is the visible one; `tab_is_visible` derives that from
/// the frame title instead.
fn views(windows: &Dispatch) -> Vec<View> {
    let mut out = Vec::new();
    let count = match windows.get_i4("Count") {
        Ok(count) => count,
        Err(_) => return out,
    };
    for i in 0..count {
        let window = match windows.item(i) {
            Ok(window) => window,
            Err(_) => continue,
        };
        // HWND is the tab host frame's handle: shared by all tabs of one window.
        let hwnd = window.hwnd().unwrap_or(0);
        let document = match window.get_dispatch("Document") {
            Ok(document) => document,
            Err(_) => continue,
        };
        let folder = match document.get_dispatch("Folder") {
            Ok(folder) => folder,
            Err(_) => continue,
        };
        let self_item = match folder.get_dispatch("Self") {
            Ok(self_item) => self_item,
            Err(_) => continue,
        };
        let path = match self_item.get_string("Path") {
            Ok(path) => path,
            Err(_) => continue,
        };
        let display = self_item.get_string("Name").unwrap_or_default();
        out.push(View {
            hwnd,
            path,
            display,
            window,
            document,
            folder,
        });
    }
    out
}

/// True if the handle is a real Explorer window frame.
///
/// The filter drops the desktop's background folder view, whose `Folder.Self.Path` is the
/// user's Desktop folder. `Shell.Windows` includes it whenever no Explorer window is open,
/// so without this filter a reveal of anything on the Desktop "succeeds" against the
/// desktop view and never asks the shell to open a window.
fn is_cabinet(hwnd: i64) -> bool {
    class_name(hwnd) == *"CabinetWClass"
}

fn class_name(hwnd: i64) -> OsString {
    if hwnd == 0 {
        return OsString::from("???");
    }
    let mut buffer = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd as isize, buffer.as_mut_ptr(), buffer.len() as i32) };
    if len > 0 {
        OsString::from_wide(&buffer[..len as usize])
    } else {
        OsString::from("???")
    }
}

/// The frame window's title bar text. Windows validates the handle for a foreign HWND,
/// so this is safe where `GetWindowText` on an arbitrary value would SendMessage into it.
fn window_title(hwnd: i64) -> String {
    if hwnd == 0 || unsafe { IsWindow(hwnd as isize) } == 0 {
        return String::new();
    }
    let mut buffer = [0u16; 256];
    let len = unsafe { GetWindowTextW(hwnd as isize, buffer.as_mut_ptr(), buffer.len() as i32) };
    if len > 0 {
        OsString::from_wide(&buffer[..len as usize]).to_string_lossy().into_owned()
    } else {
        String::new()
    }
}

/// True when this tab is the one the user can see.
///
/// A tabbed Explorer window titles itself with the *visible* tab and appends the rest:
/// observed on build 26100 as `桌面 和 9 个其他选项卡 - 文件资源管理器`, and as
/// `D:\path\to\folder 和 11 个其他选项卡 - 文件资源管理器` when the shell is set to show
/// full paths. Nothing in `Shell.Windows` marks the active entry, so the title is the only
/// signal; a window shows one tab, so either the display name or the path can be matched
/// against the leading segment without confusing the other tabs.
fn tab_is_visible(view: &View) -> bool {
    let title = window_title(view.hwnd).to_lowercase();
    if title.is_empty() {
        return false;
    }
    let display = view.display.to_string_lossy().to_lowercase();
    let path = String::from_utf16_lossy(&normalized(&view.path));
    titles_tab(&title, &display) || titles_tab(&title, &path)
}

/// Whether `title` opens with the whole of `name`.
///
/// The boundary matters: matching `D:\probe\p1` against the title of the tab
/// `D:\probe\p1-keeper` would claim a hidden tab is on screen, and selecting into it then
/// looks like the command did nothing. A tab name is always followed by the appended rest of
/// the title (` 和 N 个其他选项卡 …`, or ` - 文件资源管理器` for a single-tab window).
fn titles_tab(title: &str, name: &str) -> bool {
    !name.is_empty()
        && (title == name
            || (title.starts_with(name) && title[name.len()..].starts_with(' ')))
}

/// The Explorer window currently showing `parent`. A window whose matching tab is the one
/// on screen wins over one where the folder sits in a background tab, because the caller
/// raises this frame to make the reveal visible — raising a window that is showing
/// something else would look like "nothing happened".
///
/// A window the shell creates can come up minimized (it restores the state of the last
/// closed one) or behind the caller's own window, which is why it is raised as soon as it
/// exists rather than only once the item can be selected.
pub fn front_window_for(parent: &Path) -> Option<i64> {
    let windows = shell_windows().ok()?;
    let mut shown = None;
    let mut hidden = None;
    for view in views(&windows) {
        if !is_cabinet(view.hwnd) || !same_path(&view.path, parent.as_os_str()) {
            continue;
        }
        if tab_is_visible(&view) && shown.is_none() {
            shown = Some(view.hwnd);
        }
        if hidden.is_none() {
            hidden = Some(view.hwnd);
        }
    }
    shown.or(hidden)
}

/// What to do when the containing folder is already open, but in a tab the user is not
/// looking at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Switch {
    /// Leave the tab the user is reading alone and bring the folder's own tab forward. The
    /// switch itself has to be done by the shell, and on build 26100 that costs the folder
    /// a duplicate tab.
    NewTab,
    /// Navigate the tab the user is reading to the folder, so no tab is ever switched to
    /// and none is added — the existing tab is repurposed, and whatever it showed is gone.
    Overwrite,
}

/// Select `file_name` in an Explorer tab that shows `parent`.
///
/// A tabbed Explorer window exposes one view per tab through `Shell.Windows`, all sharing
/// one HWND. `FolderView.SelectItem` reaches a background tab, but the user cannot see it
/// and nothing this shell exposes switches to an existing tab: measured on build 26100,
/// `Navigate2` on a background tab reloads that tab silently and
/// `SHOpenFolderAndSelectItems` does not switch to it either. So when `parent` is only open
/// in the background, `switch` decides the visible outcome — [`Switch::Overwrite`]
/// navigates the tab the user *is* looking at (no tab is added), [`Switch::NewTab`] reports
/// `NoView` so the caller hands the job to the shell, which is the only thing that can add
/// a tab.
/// On success returns the window's HWND, so the caller can bring it to the front.
pub fn select_in_open_view(parent: &Path, file_name: &OsStr, switch: Switch) -> Result<i64, Miss> {
    let windows = shell_windows()?;
    let show_folder = |view: &View| is_cabinet(view.hwnd) && same_path(&view.path, parent.as_os_str());
    let mut matching: Vec<View> = views(&windows).into_iter().filter(show_folder).collect();
    if matching.is_empty() {
        return Err(Miss::NoView);
    }
    let frame = matching[0].hwnd;
    if !matching.iter().any(tab_is_visible) {
        if switch == Switch::NewTab {
            return Err(Miss::Hidden);
        }
        let shown = views(&windows)
            .into_iter()
            .find(|view| view.hwnd == frame && tab_is_visible(view))
            .map(|view| navigate_tab(&view, parent))
            .unwrap_or(false);
        if !shown {
            return Err(Miss::Hidden);
        }
        // Navigation is asynchronous: re-enumerate until the folder is on screen.
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            matching = views(&windows).into_iter().filter(show_folder).collect();
            if matching.iter().any(tab_is_visible) {
                break;
            }
        }
    }
    let mut selected = false;
    for view in matching.iter().filter(|view| tab_is_visible(view)) {
        // More than one tab of the same folder in one window is possible; keep them equal.
        selected |= select_item(view, file_name);
    }
    if selected {
        Ok(frame)
    } else {
        // The folder is on screen but does not list the item yet — Explorer refreshes its
        // item list asynchronously, so a just-created file can be missing for a moment.
        // The caller retries.
        Err(Miss::ItemNotListed)
    }
}

/// Navigate the tab behind `view` to `folder`.
///
/// The call has to go to the tab's `IWebBrowser2` (the `Shell.Windows` entry itself):
/// `Navigate2` on its `Document` returns success but leaves the view alone, while on the
/// window it really shows `folder` in that tab — measured on build 26100.
fn navigate_tab(view: &View, folder: &Path) -> bool {
    let target: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    let arguments = [Variant::bstr(&target), Variant::empty(), Variant::empty()];
    matches!(
        view.window.invoke("Navigate2", &arguments),
        Ok(result) if result.vt == VT_EMPTY
    )
}

/// Ask the shell to show `folder` in the tab that already holds it.
///
/// `SHOpenFolderAndSelectItems` selects inside a background tab and leaves it there, and
/// nothing else the shell exposes switches to it, so this uses the shell's own command line
/// — measured on build 26100 to activate the existing tab of `folder` without adding one and
/// without touching the tab the user was reading. It is strictly a *visibility* assist: no
/// `/select,` prefix is passed, because that form duplicates the tab and, as measured long
/// ago, does not select. Selection stays with `select_item`, on the tab this just brought
/// forward. The path goes to `explorer.exe` as one argument, so quoting is the OS's, not a
/// hand-built command line.
pub fn activate_tab_by_path(folder: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(folder).spawn();
}

fn select_item(view: &View, file_name: &OsStr) -> bool {
    let items = match view.folder.get_dispatch("Items") {
        Ok(items) => items,
        Err(_) => return false,
    };
    let item_count = match items.get_i4("Count") {
        Ok(item_count) => item_count,
        Err(_) => return false,
    };
    for j in 0..item_count {
        let entry = match items.item(j) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let name = match entry.get_string("Name") {
            Ok(name) => name,
            Err(_) => continue,
        };
        if !same_path(&name, file_name) {
            continue;
        }
        // Hand entry's reference to the argument variant; forget skips the
        // local release so the reference count is unchanged.
        let argument = Variant::dispatch(entry.0);
        std::mem::forget(entry);
        return view
            .document
            .invoke("SelectItem", &[argument, Variant::i4(SVSI_SELECT_AND_SHOW)])
            .is_ok();
    }
    false
}

/// Best effort: a console process's direct attempt to grab the foreground is often refused
/// by Windows, and failing here merely leaves the window where it was.
pub fn bring_to_front(hwnd: i64) {
    if hwnd == 0 {
        return;
    }
    unsafe {
        if IsIconic(hwnd as isize) != 0 {
            // A minimized window would otherwise come to the front as an empty taskbar hop.
            ShowWindow(hwnd as isize, 9); // SW_RESTORE
        }
        let foreground = GetForegroundWindow();
        let self_thread = GetCurrentThreadId();
        let other_thread = GetWindowThreadProcessId(foreground, ptr::null_mut());
        let attached = other_thread != 0
            && other_thread != self_thread
            && AttachThreadInput(self_thread, other_thread, 1) != 0;
        BringWindowToTop(hwnd as isize);
        let shown = SetForegroundWindow(hwnd as isize);
        if attached {
            AttachThreadInput(self_thread, other_thread, 0);
        }
        // If the window still refused the foreground, at least stack it on top.
        if shown == 0 {
            ShowWindow(hwnd as isize, 5); // SW_SHOW
        }
    }
}

#[link(name = "user32")]
extern "system" {
    fn SetForegroundWindow(hwnd: isize) -> i32;
    fn BringWindowToTop(hwnd: isize) -> i32;
    fn ShowWindow(hwnd: isize, command: i32) -> i32;
    fn IsIconic(hwnd: isize) -> i32;
    fn GetForegroundWindow() -> isize;
    fn GetWindowThreadProcessId(hwnd: isize, process_id: *mut u32) -> u32;
    fn GetCurrentThreadId() -> u32;
    fn AttachThreadInput(current: u32, other: u32, attach: i32) -> i32;
    fn GetClassNameW(hwnd: isize, buffer: *mut u16, max_len: i32) -> i32;
    fn IsWindow(hwnd: isize) -> i32;
    fn GetWindowTextW(hwnd: isize, buffer: *mut u16, max_len: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::same_path;
    use std::ffi::OsStr;

    #[test]
    fn path_match_ignores_case_and_trailing_separators() {
        assert!(same_path(OsStr::new(r"C:\Dir One"), OsStr::new(r"c:\dir one\")));
        assert!(same_path(OsStr::new(r"C:\Dir\"), OsStr::new(r"C:\Dir")));
        assert!(!same_path(OsStr::new(r"C:\Dir"), OsStr::new(r"C:\Dir2")));
        assert!(!same_path(OsStr::new(r"C:\Dir"), OsStr::new(r"D:\Dir")));
    }

    #[test]
    fn path_match_survives_cjk_and_spaces() {
        assert!(same_path(
            OsStr::new(r"C:\Users\桌面\中文 文件.txt"),
            OsStr::new(r"c:\users\桌面\中文 文件.TXT")
        ));
        assert!(!same_path(
            OsStr::new(r"C:\Users\桌面\文件.txt"),
            OsStr::new(r"C:\Users\桌面\文件2.txt")
        ));
    }

    #[test]
    fn drive_root_still_matches_after_separator_trim() {
        assert!(same_path(OsStr::new(r"C:\"), OsStr::new("C:")));
    }

    #[test]
    fn visible_tab_title_matches_full_path_and_leaf_name() {
        // The shell titles the window with the full path or with the tab's display name,
        // depending on the "show full paths" option; either way the match is at the front.
        let by_path = r"d:\probe\目标 目录, x 和 3 个其他选项卡 - 文件资源管理器";
        let by_name = "目标 目录, x 和 3 个其他选项卡 - 文件资源管理器";
        assert!(super::titles_tab(by_path, r"d:\probe\目标 目录, x"));
        assert!(super::titles_tab(by_name, "目标 目录, x"));
        assert!(super::titles_tab("桌面 - 文件资源管理器", "桌面"));
    }

    #[test]
    fn a_prefix_of_the_visible_tab_is_not_treated_as_visible() {
        // Selecting into a tab this rule would call visible is invisible to the user.
        let title = r"d:\probe\p1-keeper 和 1 个其他选项卡 - 文件资源管理器";
        assert!(!super::titles_tab(title, r"d:\probe\p1"));
        assert!(!super::titles_tab(title, "p1"));
        assert!(super::titles_tab(title, r"d:\probe\p1-keeper"));
    }

    #[test]
    fn empty_name_never_matches_the_title() {
        assert!(!super::titles_tab("some tab - 文件资源管理器", ""));
    }
}
