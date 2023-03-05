//! This crate provides two powerful proc macros, [`#[export_tokens]`](`macro@export_tokens`)
//! and [`import_tokens!`]. When used in tandem, these two macros allow you to mark items in
//! other files (and even in other crates, as long as you can modify the source code) for
//! export. The tokens of these items can then be imported by the [`import_tokens!`] macro using
//! the path to an item you have exported.
//!
//! Two advanced macros, [`import_tokens_indirect!`] and [`read_namespace!`] are also provided
//! when the "indirect" feature is enabled. These macro are capable of going across crate
//! boundaries without complicating your dependencies and can return collections of tokens
//! based on a shared common prefix.
//!
//! Among other things, the patterns introduced by Macro Magic, and in particular by the
//! "indirect" feature be used to implement safe and efficient coordination and communication
//! between macro invocations in the same file, and even across different files and different
//! crates. This crate officially supercedes my previous effort at achieving this,
//! [macro_state](https://crates.io/crates/macro_state), which was designed to allow for
//! building up and making use of state information across multiple macro invocations. All of
//! the things you can do with `macro_state` you can also achieve with this crate, albeit with
//! slightly different patterns.
//!
//! Macro Magic is designed to work with stable Rust.

pub use macro_magic_macros::*;

#[doc(hidden)]
pub mod __private {
    pub use syn::__private::TokenStream2;
}
