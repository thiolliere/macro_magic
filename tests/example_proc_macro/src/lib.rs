extern crate proc_macro;
use proc_macro::TokenStream;

use macro_magic::*;

#[proc_macro]
pub fn example_macro(_tokens: TokenStream) -> TokenStream {
    import_tokens!(example_crate::add2).into()
}

#[proc_macro]
pub fn example_macro2(_tokens: TokenStream) -> TokenStream {
    import_tokens!(example_crate::cool_types).into()
}

#[proc_macro]
pub fn example_macro3(_tokens: TokenStream) -> TokenStream {
    import_tokens_indirect!(example_crate2::mult).into()
}

#[proc_macro]
pub fn example_macro4(_tokens: TokenStream) -> TokenStream {
    import_tokens_indirect!(BadBad<T>).into()
}
