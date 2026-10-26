//! reveal's own declarations of the shell entry points, so the crate needs no dependency.
//!
//! Signatures are taken from the SDK headers; `*mut c_void` stands in for the `ITEMIDLIST`
//! and `PCUITEMID` pointer typedefs, and an HRESULT is kept as the `i32` the ABI defines.

use std::os::raw::c_void;

// Compiled even when `--features official-bindings` selects the other flavour: the cross-check test
// compares the two, so an unused declaration here is a second opinion, not dead code.
#[allow(dead_code)]
#[link(name = "shell32")]
extern "system" {
    pub fn SHParseDisplayName(
        psz_name: *const u16,
        pbc: *const c_void,
        ppidl: *mut *mut c_void,
        sfgao_in: u32,
        psfgao_out: *mut u32,
    ) -> i32;
    pub fn SHOpenFolderAndSelectItems(
        pidl_folder: *const c_void,
        cidl: u32,
        apidl: *const *const c_void,
        dw_flags: u32,
    ) -> i32;
    pub fn ILFindLastID(pidl: *const c_void) -> *mut c_void;
    pub fn ILRemoveLastID(pidl: *mut c_void) -> i32;
    pub fn ILFree(pidl: *mut c_void);
}

#[link(name = "ole32")]
extern "system" {
    pub fn CoInitializeEx(pv_reserved: *const c_void, coinit: u32) -> i32;
    pub fn CoUninitialize();
}
