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

// Reachable in the default build (it *is* the binding there) and under `cargo test` with the
// feature on, because the cross-check test measures the official flavour against these
// declarations. A release build with `official-bindings` has no use for them and does not
// compile them.
#[cfg(any(not(feature = "official-bindings"), test))]
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
