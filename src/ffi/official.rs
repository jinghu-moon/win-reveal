//! The same shell entry points, taken from the official `windows` crate's declarations.
//!
//! Selected with `--features official-bindings`. Every wrapper keeps [`super::native`]'s signature —
//! pointers stay `c_void`, an HRESULT stays `i32` — so the calling code does not have two
//! versions to keep in step; only the origin of the signature changes. Each function is
//! exactly as unsafe as the Win32 call it forwards to.
#![allow(non_snake_case)] // the names mirror the Win32 exports, as the native flavour's do

use std::ffi::c_void;
use std::ptr;
use windows::core::PCWSTR;
use windows::Win32::System::Com::{self as com, IBindCtx, COINIT};
use windows::Win32::UI::Shell::{self, Common::ITEMIDLIST};

/// The binding context is always the null one reveal passes, which the crate spells `None`.
pub unsafe fn SHParseDisplayName(
    psz_name: *const u16,
    _pbc: *const c_void,
    ppidl: *mut *mut c_void,
    sfgao_in: u32,
    psfgao_out: *mut u32,
) -> i32 {
    let mut parsed: *mut ITEMIDLIST = ptr::null_mut();
    let outcome = Shell::SHParseDisplayName(
        PCWSTR(psz_name),
        None::<&IBindCtx>,
        &mut parsed,
        sfgao_in,
        if psfgao_out.is_null() { None } else { Some(psfgao_out) },
    );
    *ppidl = parsed as *mut c_void;
    code_of(outcome)
}

pub unsafe fn SHOpenFolderAndSelectItems(
    pidl_folder: *const c_void,
    cidl: u32,
    apidl: *const *const c_void,
    dw_flags: u32,
) -> i32 {
    // The crate derives `cidl` from the slice, so rebuild it from the raw pair.
    let items = if apidl.is_null() {
        None
    } else {
        Some(unsafe { std::slice::from_raw_parts(apidl as *const *const ITEMIDLIST, cidl as usize) })
    };
    code_of(Shell::SHOpenFolderAndSelectItems(pidl_folder as *const ITEMIDLIST, items, dw_flags))
}

pub unsafe fn ILFindLastID(pidl: *const c_void) -> *mut c_void {
    Shell::ILFindLastID(pidl as *const ITEMIDLIST) as *mut c_void
}

pub unsafe fn ILRemoveLastID(pidl: *mut c_void) -> i32 {
    Shell::ILRemoveLastID((!pidl.is_null()).then_some(pidl as *mut ITEMIDLIST)).0
}

pub unsafe fn ILFree(pidl: *mut c_void) {
    Shell::ILFree((!pidl.is_null()).then_some(pidl as *const ITEMIDLIST));
}

pub unsafe fn CoInitializeEx(pv_reserved: *const c_void, coinit: u32) -> i32 {
    com::CoInitializeEx((!pv_reserved.is_null()).then_some(pv_reserved), COINIT(coinit as i32)).0
}

pub unsafe fn CoUninitialize() {
    com::CoUninitialize();
}

/// A crate `Result` back to the HRESULT number the raw signatures return.
fn code_of(outcome: windows::core::Result<()>) -> i32 {
    match outcome {
        Ok(()) => 0,
        Err(error) => error.code().0,
    }
}
