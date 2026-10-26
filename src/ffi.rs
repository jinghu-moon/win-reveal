//! The Win32 entry points reveal calls, in two interchangeable flavours.
//!
//! * [`native`] declares the handful of shell32/ole32 exports by hand. It is the default,
//!   and it is what keeps the shipped binary free of anything but Windows itself.
//! * `official` — selected with `--features official-bindings` — routes the same calls through
//!   the official `windows` crate's declarations, for a project that already depends on that
//!   crate and would rather share its signatures than trust ours.
//!
//! Only these declarations differ; there is one algorithm and one set of call sites. The
//! `IDispatch` late-binding client in [`crate::automation`] stays hand-written either way:
//! Explorer's folder view is reached through `IDispatch`, whose `VARIANT` and
//! `GetIDsOfNames` plumbing is this crate's own code, not a binding of a documented
//! interface.

#[cfg(feature = "official-bindings")]
mod official;

// Compiled in both flavours: the cross-check test measures whichever flavour is active
// against these declarations, so they have to stay reachable even when the feature picks
// `official`.
#[cfg_attr(all(feature = "official-bindings", not(test)), allow(dead_code))]
pub(crate) mod native;

#[cfg(feature = "official-bindings")]
pub(crate) use official::{
    CoInitializeEx, CoUninitialize, ILFindLastID, ILFree, ILRemoveLastID,
    SHOpenFolderAndSelectItems, SHParseDisplayName,
};
#[cfg(not(feature = "official-bindings"))]
pub(crate) use native::{
    CoInitializeEx, CoUninitialize, ILFindLastID, ILFree, ILRemoveLastID,
    SHOpenFolderAndSelectItems, SHParseDisplayName,
};
