//! This crate contains most of the internal implementation of the macros in the
//! `macro_magic_macros` crate. For the most part, the proc macros in `macro_magic_macros` just
//! call their respective `_internal` variants in this crate.

#![no_std]
extern crate alloc;
use core::fmt::Display;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

#[cfg(feature = "pretty_print")]
use libc_print::libc_println as println;

use derive_syn_parse::Parse;
use macro_magic_core_macros::*;
use proc_macro2::{Punct, Spacing, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::Nothing, parse2, parse_quote, spanned::Spanned, token::Comma, Attribute, Error, FnArg,
    Ident, Item, ItemFn, LitStr, Pat, Path, Result, Token, Visibility,
};

pub const MACRO_MAGIC_ROOT: &'static str = get_macro_magic_root!();

/// Private module containing custom keywords used for parsing in this crate
mod keywords {
    use syn::custom_keyword;

    custom_keyword!(proc_macro_attribute);
    custom_keyword!(proc_macro);
    custom_keyword!(proc_macro_derive);
}

/// Used to parse args that were passed to [`forward_tokens_internal`].
///
/// You shouldn't need to use this directly.
#[derive(Parse)]
pub struct ForwardTokensArgs {
    /// The path of the item whose tokens are being forwarded
    pub source: Path,
    _comma1: Comma,
    /// The path of the macro that will receive the forwarded tokens
    pub target: Path,
    _comma2: Option<Comma>,
    #[parse_if(_comma2.is_some())]
    pub mm_path: Option<Path>,
    _comma3: Option<Comma>,
    #[parse_if(_comma3.is_some())]
    /// Optional extra data that can be passed as a [`struct@LitStr`]. This is how
    /// [`import_tokens_attr_internal`] passes the item the attribute macro is attached to, but
    /// this can be repurposed for other things potentially as [`str`] could encode anything.
    pub extra: Option<LitStr>,
}

/// Used to parse args that were passed to [`forward_tokens_inner_internal`].
///
/// You shouldn't need to use this directly.
#[derive(Parse)]
pub struct ForwardedTokens {
    /// The path of the macro that will receive the forwarded tokens
    pub target_path: Path,
    _comma1: Comma,
    /// The item whose tokens are being forwarded
    pub item: Item,
    _comma2: Option<Comma>,
    #[parse_if(_comma2.is_some())]
    /// Optional extra data that can be passed as a [`struct@LitStr`]. This is how
    /// [`import_tokens_attr_internal`] passes the item the attribute macro is attached to, but
    /// this can be repurposed for other things potentially as [`str`] could encode anything.
    pub extra: Option<LitStr>,
}

/// Used to parse args passed to the inner pro macro auto-generated by
/// [`import_tokens_attr_internal`].
///
/// You shouldn't need to use this directly.
#[derive(Parse)]
pub struct AttrItemWithExtra {
    pub imported_item: Item,
    _comma: Comma,
    pub extra: LitStr,
}

/// Used to parse the args for the [`import_tokens_internal`] function.
///
/// You shouldn't need to use this directly.
#[derive(Parse)]
pub struct ImportTokensArgs {
    _let: Token![let],
    pub tokens_var_ident: Ident,
    _eq: Token![=],
    pub source_path: Path,
}

/// Used to parse the args for the [`import_tokens_inner_internal`] function.
///
/// You shouldn't need to use this directly.
#[derive(Parse)]
pub struct ImportedTokens {
    pub tokens_var_ident: Ident,
    _comma: Comma,
    pub item: Item,
}

#[derive(Parse)]
pub struct BasicUseStmt {
    #[call(Attribute::parse_outer)]
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    _use: Token![use],
    pub path: Path,
    _semi: Token![;],
}

/// Delineates the different types of proc macro
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum ProcMacroType {
    /// Corresponds with `#[proc_macro]`
    Normal,
    /// Corresponds with `#[proc_macro_attribute]`
    Attribute,
    /// Corresponds with `#[proc_macro_derive]`
    Derive,
}

impl ProcMacroType {
    /// Gets the `&'static str` representation of this proc macro type
    pub fn to_str(&self) -> &'static str {
        match self {
            ProcMacroType::Normal => "#[proc_macro]",
            ProcMacroType::Attribute => "#[proc_macro_attribute]",
            ProcMacroType::Derive => "#[proc_macro_derive]",
        }
    }

    /// Gets the [`Attribute`] representation of this proc macro type
    pub fn to_attr(&self) -> Attribute {
        match self {
            ProcMacroType::Normal => parse_quote!(#[proc_macro]),
            ProcMacroType::Attribute => parse_quote!(#[proc_macro_attribute]),
            ProcMacroType::Derive => parse_quote!(#[proc_macro_derive]),
        }
    }
}

/// Should be implemented by structs that will be passed to `#[with_custom_parsing(..)]`. Such
/// structs should also implement [`syn::parse::Parse`].
///
/// ## Example
///
/// ```ignore
/// #[derive(derive_syn_parse::Parse)]
/// struct CustomParsingA {
///     foreign_path: syn::Path,
///     _comma: syn::token::Comma,
///     custom_path: syn::Path,
/// }
///
/// impl ToTokens for CustomParsingA {
///     fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
///         tokens.extend(self.foreign_path.to_token_stream());
///         tokens.extend(self._comma.to_token_stream());
///         tokens.extend(self.custom_path.to_token_stream());
///     }
/// }
///
/// impl ForeignPath for CustomParsingA {
///     fn foreign_path(&self) -> &syn::Path {
///         &self.foreign_path
///     }
/// }
/// ```
pub trait ForeignPath {
    fn foreign_path(&self) -> &syn::Path;
}

#[derive(Clone)]
pub struct ProcMacro {
    /// The underlying proc macro function definition
    pub proc_fn: ItemFn,
    /// Specified the type of this proc macro, i.e. attribute vs normal vs derive
    pub macro_type: ProcMacroType,
    /// Specifies the [`struct@Ident`] for the `tokens` parameter of this proc macro function
    /// definition. For normal and derive macros this is the only parameter, and for attribute
    /// macros this is the second parameter.
    pub tokens_ident: Ident,
    /// Specifies the [`struct@Ident`] for the `attr` parameter of this proc macro function
    /// definition, if it is an attribute macro. Otherwise this will be set to [`None`].
    pub attr_ident: Option<Ident>,
}

impl ProcMacro {
    /// Constructs a [`ProcMacro`] from anything compatible with [`TokenStream2`].
    pub fn from<T: Into<TokenStream2>>(tokens: T) -> Result<Self> {
        let proc_fn = parse2::<ItemFn>(tokens.into())?;
        let Visibility::Public(_) = proc_fn.vis else { return Err(Error::new(proc_fn.vis.span(), "Visibility must be public")) };
        let mut macro_type: Option<ProcMacroType> = None;
        if proc_fn
            .attrs
            .iter()
            .find(|attr| {
                if syn::parse2::<keywords::proc_macro>(attr.path().to_token_stream()).is_ok() {
                    macro_type = Some(ProcMacroType::Normal);
                } else if syn::parse2::<keywords::proc_macro_attribute>(
                    attr.path().to_token_stream(),
                )
                .is_ok()
                {
                    macro_type = Some(ProcMacroType::Attribute);
                } else if syn::parse2::<keywords::proc_macro>(attr.path().to_token_stream()).is_ok()
                {
                    macro_type = Some(ProcMacroType::Derive);
                }
                macro_type.is_some()
            })
            .is_none()
        {
            return Err(Error::new(
                proc_fn.sig.ident.span(),
                "can only be attached to a proc macro function definition",
            ));
        };
        let macro_type = macro_type.unwrap();

        // tokens_ident
        let Some(FnArg::Typed(tokens_arg)) = proc_fn.sig.inputs.last() else {
            unreachable!("missing tokens arg");
        };
        let Pat::Ident(tokens_ident) = *tokens_arg.pat.clone() else {
            unreachable!("invalid tokens arg");
        };
        let tokens_ident = tokens_ident.ident;

        // attr_ident (if applicable)
        let attr_ident = match macro_type {
            ProcMacroType::Attribute => {
                let Some(FnArg::Typed(attr_arg)) = proc_fn.sig.inputs.first() else {
                    unreachable!("missing attr arg");
                };
                let Pat::Ident(attr_ident) = *attr_arg.pat.clone() else {
                    unreachable!("invalid attr arg");
                };
                Some(attr_ident.ident)
            }
            _ => None,
        };
        Ok(ProcMacro {
            proc_fn,
            macro_type,
            tokens_ident,
            attr_ident,
        })
    }
}

/// Parses a proc macro function from a `TokenStream2` expecting only the specified `macro_type`
pub fn parse_proc_macro_variant<T: Into<TokenStream2>>(
    tokens: T,
    macro_type: ProcMacroType,
) -> Result<ProcMacro> {
    let proc_macro = ProcMacro::from(tokens.into())?;
    if proc_macro.macro_type != macro_type {
        let actual = proc_macro.macro_type.to_str();
        let desired = macro_type.to_str();
        return Err(Error::new(
            proc_macro.proc_fn.sig.ident.span(),
            format!(
                "expected a function definition with {} but found {} instead",
                actual, desired
            ),
        ));
    }
    Ok(proc_macro)
}

/// Convenience function that will pretty-print anything compatible with [`TokenStream2`]
/// including [`TokenStream2`], `TokenStream`, and all [`syn`] items.
///
/// Uses the `prettyplease` crate. Only built if the `pretty_print` feature is enabled.
#[cfg(feature = "pretty_print")]
pub fn pretty_print<T: Into<TokenStream2> + Clone>(tokens: &T) {
    let tokens = (*tokens).clone();
    println!(
        "\n\n{}\n\n",
        prettyplease::unparse(&syn::parse_file(tokens.into().to_string().as_str()).unwrap())
    );
}

/// Safely access the `macro_magic` root based on the `MACRO_MAGIC_ROOT` env var, which
/// defaults to `::macro_magic`, but can be configured via the `[env]` section of
/// `.cargo/config.toml`
pub fn macro_magic_root() -> Path {
    parse2::<Path>(
        MACRO_MAGIC_ROOT
            .parse::<TokenStream2>()
            .expect("environment var `MACRO_MAGIC_ROOT` must parse to a valid TokenStream2"),
    )
    .expect("environment variable `MACRO_MAGIC_ROOT` must parse to a valid syn::Path")
}

/// Safely access a subpath of `macro_magic::__private`
pub fn private_path<T: Into<TokenStream2> + Clone>(subpath: &T) -> Path {
    let subpath = subpath.clone().into();
    let root = macro_magic_root();
    parse_quote!(#root::__private::#subpath)
}

/// Safely access a subpath of `macro_magic`
pub fn macro_magic_path<T: Into<TokenStream2> + Clone>(subpath: &T) -> Path {
    let subpath = subpath.clone().into();
    let root = macro_magic_root();
    parse_quote! {
        #root::#subpath
    }
}

/// Returns the specified string in snake_case
pub fn to_snake_case(input: impl Into<String>) -> String {
    let input: String = input.into();
    if input.len() == 0 {
        return input.into();
    }
    let mut prev_lower = input.chars().next().unwrap().is_lowercase();
    let mut prev_whitespace = true;
    let mut first = true;
    let mut output: Vec<char> = Vec::new();
    for c in input.chars() {
        if c == '_' {
            prev_whitespace = true;
            output.push('_');
            continue;
        }
        if !c.is_ascii_alphanumeric() && c != '_' && !c.is_whitespace() {
            continue;
        }
        if !first && c.is_whitespace() || c == '_' {
            if !prev_whitespace {
                output.push('_');
            }
            prev_whitespace = true;
        } else {
            let current_lower = c.is_lowercase();
            if ((prev_lower != current_lower && prev_lower)
                || (prev_lower == current_lower && !prev_lower))
                && !first
                && !prev_whitespace
            {
                output.push('_');
            }
            output.push(c.to_ascii_lowercase());
            prev_lower = current_lower;
            prev_whitespace = false;
        }
        first = false;
    }
    output.iter().collect::<String>()
}

/// Converts a string-like value (via [`Display`]) such that the sequence `~~` is safely escaped
/// so that `~~` can be used as a list delimiter.
///
/// Used by [`forward_tokens_internal`] to escape items appearing in the `extra` variable.
pub fn escape_extra<T: Display>(extra: T) -> String {
    extra
        .to_string()
        .replace("\\", "\\\\")
        .replace("~~", "\\~\\~")
}

/// Unescapes a `String` that has been escaped via [`escape_extra`].
///
/// Used by [`forward_tokens_internal`] to unescape items appearing in the `extra` variable.
pub fn unescape_extra<T: Display>(extra: T) -> String {
    extra
        .to_string()
        .replace("\\\\", "\\")
        .replace("\\~\\~", "~~")
}

/// "Flattens" an [`struct@Ident`] by converting it to snake case.
///
/// Used by [`export_tokens_macro_ident`].
pub fn flatten_ident(ident: &Ident) -> Ident {
    Ident::new(to_snake_case(ident.to_string()).as_str(), ident.span())
}

/// Produces the full path for the auto-generated callback-based decl macro that allows us to
/// forward tokens across crate boundaries.
///
/// Used by [`export_tokens_internal`] and several other functions.
pub fn export_tokens_macro_ident(ident: &Ident) -> Ident {
    let ident = flatten_ident(&ident);
    let ident_string = format!("__export_tokens_tt_{}", ident.to_token_stream().to_string());
    Ident::new(ident_string.as_str(), Span::call_site())
}

/// The internal code behind the `#[export_tokens]` attribute macro.
///
/// The `attr` variable contains the tokens for the optional naming [`struct@Ident`] (necessary
/// on [`Item`]s that don't have an inherent [`struct@Ident`]), and the `tokens` variable is
/// the tokens for the [`Item`] the attribute macro can be attached to. The `attr` variable can
/// be blank tokens for supported items, which include every valid [`syn::Item`] except for
/// [`syn::ItemForeignMod`], [`syn::ItemUse`], [`syn::ItemImpl`], and [`Item::Verbatim`], which
/// all require `attr` to be specified.
///
/// An empty [`TokenStream2`] is sufficient for opting out of using `attr`
pub fn export_tokens_internal<T: Into<TokenStream2>, E: Into<TokenStream2>>(
    attr: T,
    tokens: E,
    emit: bool,
) -> Result<TokenStream2> {
    let attr = attr.into();
    let item: Item = parse2(tokens.into())?;
    let ident = match item.clone() {
        Item::Const(item_const) => Some(item_const.ident),
        Item::Enum(item_enum) => Some(item_enum.ident),
        Item::ExternCrate(item_extern_crate) => Some(item_extern_crate.ident),
        Item::Fn(item_fn) => Some(item_fn.sig.ident),
        Item::Macro(item_macro) => item_macro.ident, // note this one might not have an Ident as well
        Item::Mod(item_mod) => Some(item_mod.ident),
        Item::Static(item_static) => Some(item_static.ident),
        Item::Struct(item_struct) => Some(item_struct.ident),
        Item::Trait(item_trait) => Some(item_trait.ident),
        Item::TraitAlias(item_trait_alias) => Some(item_trait_alias.ident),
        Item::Type(item_type) => Some(item_type.ident),
        Item::Union(item_union) => Some(item_union.ident),
        // Item::ForeignMod(item_foreign_mod) => None,
        // Item::Use(item_use) => None,
        // Item::Impl(item_impl) => None,
        // Item::Verbatim(_) => None,
        _ => None,
    };
    let ident = match ident {
        Some(ident) => {
            if let Ok(_) = parse2::<Nothing>(attr.clone()) {
                ident
            } else {
                parse2::<Ident>(attr)?
            }
        }
        None => parse2::<Ident>(attr)?,
    };
    let ident = export_tokens_macro_ident(&ident);
    let item_emit = match emit {
        true => quote! {
            #[allow(unused)]
            #item
        },
        false => quote!(),
    };
    let output = quote! {
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #ident {
            // arm with extra support (used by attr)
            ($(::)?$($tokens_var:ident)::*, $(::)?$($callback:ident)::*, $extra:expr) => {
                $($callback)::*! {
                    $($tokens_var)::*,
                    #item,
                    $extra
                }
            };
            // regular arm (used by proc, import_tokens, etc)
            ($(::)?$($tokens_var:ident)::*, $(::)?$($callback:ident)::*) => {
                $($callback)::*! {
                    $($tokens_var)::*,
                    #item
                }
            };
        }
        #item_emit
    };
    // pretty_print(&output);
    Ok(output)
}

/// Internal implementation of `export_tokens_alias!`. Allows creating a renamed/rebranded
/// macro that does the same thing as `#[export_tokens]`
pub fn export_tokens_alias_internal<T: Into<TokenStream2>>(
    tokens: T,
    emit: bool,
) -> Result<TokenStream2> {
    let alias = parse2::<Ident>(tokens.into())?;
    let export_tokens_internal_path = macro_magic_path(&quote!(mm_core::export_tokens_internal));
    Ok(quote! {
        #[proc_macro_attribute]
        pub fn #alias(attr: proc_macro::TokenStream, tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
            match #export_tokens_internal_path(attr, tokens, #emit) {
                Ok(tokens) => tokens.into(),
                Err(err) => err.to_compile_error().into(),
            }
        }
    })
}

/// The internal implementation for the `import_tokens` macro.
///
/// You can call this in your own proc macros to make use of the `import_tokens` functionality
/// directly, though this approach is limited. The arguments should be a [`TokenStream2`] that
/// can parse into an [`ImportTokensArgs`] successfully. That is a valid `let` variable
/// declaration set to equal a path where an `#[export_tokens]` with the specified ident can be
/// found.
///
/// ### Example:
/// ```
/// use macro_magic_core::*;
/// use quote::quote;
///
/// let some_ident = quote!(my_tokens);
/// let some_path = quote!(other_crate::exported_item);
/// let tokens = import_tokens_internal(quote!(let #some_ident = other_crate::ExportedItem)).unwrap();
/// assert_eq!(
///     tokens.to_string(),
///     "other_crate :: __export_tokens_tt_exported_item ! { my_tokens , \
///     :: macro_magic :: __private :: import_tokens_inner }");
/// ```
/// If these tokens were emitted as part of a proc macro, they would expand to a variable
/// declaration like:
/// ```ignore
/// let my_tokens: TokenStream2;
/// ```
/// where `my_tokens` contains the tokens of `ExportedItem`.
pub fn import_tokens_internal<T: Into<TokenStream2>>(tokens: T) -> Result<TokenStream2> {
    let args = parse2::<ImportTokensArgs>(tokens.into())?;
    let Some(source_ident_seg) = args.source_path.segments.last() else { unreachable!("must have at least one segment") };
    let source_ident_seg = export_tokens_macro_ident(&source_ident_seg.ident);
    let source_path = if args.source_path.segments.len() > 1 {
        let Some(crate_seg) = args.source_path.segments.first() else {
            unreachable!("path has at least two segments, so there is a first segment");
        };
        quote!(#crate_seg::#source_ident_seg)
    } else {
        quote!(#source_ident_seg)
    };
    let inner_macro_path = private_path(&quote!(import_tokens_inner));
    let tokens_var_ident = args.tokens_var_ident;
    Ok(quote! {
        #source_path! { #tokens_var_ident, #inner_macro_path }
    })
}

/// The internal implementation for the `import_tokens_inner` macro.
///
/// You shouldn't need to call this in any circumstances but it is provided just in case.
pub fn import_tokens_inner_internal<T: Into<TokenStream2>>(tokens: T) -> Result<TokenStream2> {
    let parsed = parse2::<ImportedTokens>(tokens.into())?;
    let tokens_string = parsed.item.to_token_stream().to_string();
    let ident = parsed.tokens_var_ident;
    let token_stream_2 = private_path(&quote!(TokenStream2));
    Ok(quote! {
        let #ident = #tokens_string.parse::<#token_stream_2>().expect("failed to parse quoted tokens");
    })
}

/// The internal implementation for the `forward_tokens` macro.
///
/// You shouldn't need to call this in any circumstances but it is provided just in case.
pub fn forward_tokens_internal<T: Into<TokenStream2>>(tokens: T) -> Result<TokenStream2> {
    let args = parse2::<ForwardTokensArgs>(tokens.into())?;
    let mm_path = match args.mm_path {
        Some(path) => path,
        None => macro_magic_root(),
    };
    let Some(source_ident_seg) = args.source.segments.last() else { unreachable!("must have at least one segment") };
    let source_ident_seg = export_tokens_macro_ident(&source_ident_seg.ident);
    let source_path = if args.source.segments.len() > 1 {
        let Some(crate_seg) = args.source.segments.first() else {
            unreachable!("path has at least two segments, so there is a first segment");
        };
        quote!(#crate_seg::#source_ident_seg)
    } else {
        quote!(#source_ident_seg)
    };
    let target_path = args.target;
    if let Some(extra) = args.extra {
        Ok(quote! {
            #source_path! {
                #target_path,
                #mm_path::__private::forward_tokens_inner,
                #extra
            }
        })
    } else {
        Ok(quote! {
            #source_path! { #target_path, #mm_path::__private::forward_tokens_inner }
        })
    }
}

/// Used by [`forward_tokens_internal`].
pub fn forward_tokens_inner_internal<T: Into<TokenStream2>>(tokens: T) -> Result<TokenStream2> {
    let parsed = parse2::<ForwardedTokens>(tokens.into())?;
    let target_path = parsed.target_path;
    let imported_tokens = parsed.item;
    let combined_tokens = match parsed.extra {
        Some(extra) => quote! {
            #imported_tokens,
            #extra
        },
        None => quote!(#imported_tokens),
    };
    Ok(quote! {
        #target_path! {
            #combined_tokens
        }
    })
}

/// The internal implementation for the `#[with_custom_parsing(..)` attribute macro.
///
/// Note that this implementation just does parsing and re-orders the attributes of the
/// attached proc macro attribute definition such that the `#[import_tokens_attr]` attribute
/// comes before this attribute. The real implementation for `#[with_custom_parsing(..)]` can
/// be found in [`import_tokens_attr_internal`]. The purpose of this is to allow programmers to
/// use either ordering and still have the proper compiler errors when something is invalid.
///
/// The `import_tokens_att_name` argument is used when generating error messages and matching
/// against the `#[import_tokens_attr]` macro this is to be used with. If you use a
/// renamed/rebranded version of `#[import_tokens_attr]`, you should change this value to match
/// the name of your macro.
pub fn with_custom_parsing_internal<T1: Into<TokenStream2>, T2: Into<TokenStream2>>(
    attr: T1,
    tokens: T2,
    import_tokens_attr_name: &'static str,
) -> Result<TokenStream2> {
    // verify that we are attached to a valid #[import_tokens_attr] proc macro def
    let proc_macro = parse_proc_macro_variant(tokens, ProcMacroType::Attribute)?;
    if proc_macro
        .proc_fn
        .attrs
        .iter()
        .find(|attr| {
            if let Some(seg) = attr.meta.path().segments.last() {
                return seg.ident == import_tokens_attr_name;
            }
            false
        })
        .is_none()
    {
        return Err(Error::new(
            Span::call_site(),
            format!(
                "Can only be attached to an attribute proc macro marked with `#[{}]`",
                import_tokens_attr_name
            ),
        ));
    }

    // ensure there is only one `#[with_custom_parsing]`
    if proc_macro
        .proc_fn
        .attrs
        .iter()
        .find(|attr| {
            if let Some(seg) = attr.meta.path().segments.last() {
                return seg.ident == "with_custom_parsing_internal";
            }
            false
        })
        .is_some()
    {
        return Err(Error::new(
            Span::call_site(),
            "Only one instance of #[with_custom_parsing] can be attached at a time.",
        ));
    }

    // parse attr to ensure it is a Path
    let custom_path = parse2::<Path>(attr.into())?;

    // emit original item unchanged now that parsing has passed
    let mut item_fn = proc_macro.proc_fn;
    item_fn
        .attrs
        .push(parse_quote!(#[with_custom_parsing(#custom_path)]));

    Ok(quote!(#item_fn))
}

/// Internal implementation for the `#[import_tokens_attr]` attribute.
///
/// You shouldn't need to use this directly, but it may be useful if you wish to rebrand/rename
/// the `#[import_tokens_attr]` macro without extra indirection.
pub fn import_tokens_attr_internal<T1: Into<TokenStream2>, T2: Into<TokenStream2>>(
    attr: T1,
    tokens: T2,
) -> Result<TokenStream2> {
    let mm_override_path = match parse2::<Path>(attr.into()) {
        Ok(override_path) => override_path,
        Err(_) => macro_magic_root(),
    };
    let mm_path = macro_magic_root();
    let mut proc_macro = parse_proc_macro_variant(tokens, ProcMacroType::Attribute)?;

    // params
    let attr_ident = proc_macro.attr_ident.unwrap();
    let tokens_ident = proc_macro.tokens_ident;

    // handle custom parsing, if applicable
    let path_resolver = if let Some(index) = proc_macro.proc_fn.attrs.iter().position(|attr| {
        if let Some(seg) = attr.meta.path().segments.last() {
            return seg.ident == "with_custom_parsing";
        }
        false
    }) {
        let custom_attr = &proc_macro.proc_fn.attrs[index];
        let custom_struct_path: Path = custom_attr.parse_args()?;

        proc_macro.proc_fn.attrs.remove(index);
        quote! {
            let custom_parsed = syn::parse_macro_input!(#attr_ident as #custom_struct_path);
            let path = (&custom_parsed as &dyn ForeignPath).foreign_path();
            let _ = (&custom_parsed as &dyn quote::ToTokens);
        }
    } else {
        quote! {
            let custom_parsed = quote::quote!();
            let path = syn::parse_macro_input!(#attr_ident as syn::Path);
        }
    };

    // outer macro
    let orig_sig = proc_macro.proc_fn.sig;
    let orig_stmts = proc_macro.proc_fn.block.stmts;
    let orig_attrs = proc_macro.proc_fn.attrs;

    // inner macro
    let inner_macro_ident = format_ident!("__import_tokens_attr_{}_inner", orig_sig.ident);
    let mut inner_sig = orig_sig.clone();
    inner_sig.ident = inner_macro_ident.clone();
    inner_sig.inputs.pop().unwrap();

    let pound = Punct::new('#', Spacing::Alone);

    // final quoted tokens
    Ok(quote! {
        #(#orig_attrs)
        *
        pub #orig_sig {
            use #mm_path::__private::*;
            use #mm_path::__private::quote::ToTokens;
            use #mm_path::mm_core::*;
            let attached_item = syn::parse_macro_input!(#tokens_ident as syn::Item);
            let attached_item_str = attached_item.to_token_stream().to_string();
            #path_resolver
            let extra = format!(
                "{}~~{}~~{}",
                escape_extra(attached_item_str),
                escape_extra(path.to_token_stream().to_string().as_str()),
                escape_extra(custom_parsed.to_token_stream().to_string().as_str())
            );
            quote::quote! {
                #mm_override_path::forward_tokens! {
                    #pound path,
                    #inner_macro_ident,
                    #mm_override_path,
                    #pound extra
                }
            }.into()
        }

        #[doc(hidden)]
        #[proc_macro]
        pub #inner_sig {
            let __combined_args = #mm_path::__private::syn::parse_macro_input!(#attr_ident as #mm_path::mm_core::AttrItemWithExtra);
            let (#attr_ident, #tokens_ident) = (__combined_args.imported_item, __combined_args.extra);
            let #attr_ident: proc_macro::TokenStream = #attr_ident.to_token_stream().into();
            let (#tokens_ident, __source_path, __custom_tokens) = {
                use #mm_path::mm_core::unescape_extra;
                let extra = #tokens_ident.value();
                let mut extra_split = extra.split("~~");
                let (tokens_string, foreign_path_string, custom_parsed_string) = (
                    unescape_extra(extra_split.next().unwrap()),
                    unescape_extra(extra_split.next().unwrap()),
                    unescape_extra(extra_split.next().unwrap()),
                );
                let foreign_path: proc_macro::TokenStream = foreign_path_string.as_str().parse().unwrap();
                let tokens: proc_macro::TokenStream = tokens_string.as_str().parse().unwrap();
                let custom_parsed_tokens: proc_macro::TokenStream = custom_parsed_string.as_str().parse().unwrap();
                (tokens, foreign_path, custom_parsed_tokens)
            };
            #(#orig_stmts)
            *
        }
    })
}

/// Internal implementation for the `#[import_tokens_proc]` attribute.
///
/// You shouldn't need to use this directly, but it may be useful if you wish to rebrand/rename
/// the `#[import_tokens_proc]` macro without extra indirection.
pub fn import_tokens_proc_internal<T1: Into<TokenStream2>, T2: Into<TokenStream2>>(
    attr: T1,
    tokens: T2,
) -> Result<TokenStream2> {
    let mm_override_path = match parse2::<Path>(attr.into()) {
        Ok(override_path) => override_path,
        Err(_) => macro_magic_root(),
    };
    let mm_path = macro_magic_root();
    let proc_macro = parse_proc_macro_variant(tokens, ProcMacroType::Normal)?;

    // outer macro
    let orig_sig = proc_macro.proc_fn.sig;
    let orig_stmts = proc_macro.proc_fn.block.stmts;
    let orig_attrs = proc_macro.proc_fn.attrs;

    // inner macro
    let inner_macro_ident = format_ident!("__import_tokens_proc_{}_inner", orig_sig.ident);
    let mut inner_sig = orig_sig.clone();
    inner_sig.ident = inner_macro_ident.clone();
    inner_sig.inputs = inner_sig.inputs.iter().rev().cloned().collect();

    // params
    let tokens_ident = proc_macro.tokens_ident;

    let pound = Punct::new('#', Spacing::Alone);

    // TODO: add support for forwarding source_path for these as well

    Ok(quote! {
        #(#orig_attrs)
        *
        pub #orig_sig {
            use #mm_path::__private::*;
            use #mm_path::__private::quote::ToTokens;
            let source_path = match syn::parse::<syn::Path>(#tokens_ident) {
                Ok(path) => path,
                Err(e) => return e.to_compile_error().into(),
            };
            quote::quote! {
                #mm_override_path::forward_tokens! {
                    #pound source_path,
                    #inner_macro_ident,
                    #mm_override_path
                }
            }.into()
        }

        #[doc(hidden)]
        #[proc_macro]
        pub #inner_sig {
            #(#orig_stmts)
            *
        }
    })
}

/// Internal implementation for the `#[use_proc]` and `#[use_attr]` attribute macros
pub fn use_internal<T1: Into<TokenStream2>, T2: Into<TokenStream2>>(
    attr: T1,
    tokens: T2,
    mode: ProcMacroType,
) -> Result<TokenStream2> {
    parse2::<Nothing>(attr.into())?;
    let orig_stmt = parse2::<BasicUseStmt>(tokens.into())?;
    let orig_path = orig_stmt.path.clone();
    let orig_attrs = orig_stmt.attrs;
    let vis = orig_stmt.vis;
    let ident = &orig_stmt
        .path
        .segments
        .last()
        .expect("path must have at least one segment")
        .ident;
    let hidden_ident = match mode {
        ProcMacroType::Normal => format_ident!("__import_tokens_proc_{}_inner", ident),
        ProcMacroType::Attribute => format_ident!("__import_tokens_attr_{}_inner", ident),
        ProcMacroType::Derive => unimplemented!(),
    };
    let mut hidden_path: Path = orig_stmt.path.clone();
    hidden_path.segments.last_mut().unwrap().ident = hidden_ident;
    Ok(quote! {
        #(#orig_attrs)
        *
        #vis use #orig_path;
        #[doc(hidden)]
        #vis use #hidden_path;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_tokens_internal_missing_ident() {
        assert!(
            export_tokens_internal(quote!(), quote!(impl MyTrait for Something), true).is_err()
        );
    }

    #[test]
    fn export_tokens_internal_normal_no_ident() {
        assert!(export_tokens_internal(
            quote!(),
            quote!(
                struct MyStruct {}
            ),
            true
        )
        .unwrap()
        .to_string()
        .contains("my_struct"));
    }

    #[test]
    fn export_tokens_internal_normal_ident() {
        assert!(export_tokens_internal(
            quote!(some_name),
            quote!(
                struct Something {}
            ),
            true,
        )
        .unwrap()
        .to_string()
        .contains("some_name"));
    }

    #[test]
    fn export_tokens_internal_generics_no_ident() {
        assert!(export_tokens_internal(
            quote!(),
            quote!(
                struct MyStruct<T> {}
            ),
            true,
        )
        .unwrap()
        .to_string()
        .contains("__export_tokens_tt_my_struct"));
    }

    #[test]
    fn export_tokens_internal_bad_ident() {
        assert!(export_tokens_internal(
            quote!(Something<T>),
            quote!(
                struct MyStruct {}
            ),
            true,
        )
        .is_err());
        assert!(export_tokens_internal(
            quote!(some::path),
            quote!(
                struct MyStruct {}
            ),
            true,
        )
        .is_err());
    }

    #[test]
    fn test_export_tokens_no_emit() {
        assert!(export_tokens_internal(
            quote!(some_name),
            quote!(
                struct Something {}
            ),
            false,
        )
        .unwrap()
        .to_string()
        .contains("some_name"));
    }

    #[test]
    fn import_tokens_internal_simple_path() {
        assert!(
            import_tokens_internal(quote!(let tokens = my_crate::SomethingCool))
                .unwrap()
                .to_string()
                .contains("__export_tokens_tt_something_cool")
        );
    }

    #[test]
    fn import_tokens_internal_flatten_long_paths() {
        assert!(import_tokens_internal(
            quote!(let tokens = my_crate::some_mod::complex::SomethingElse)
        )
        .unwrap()
        .to_string()
        .contains("__export_tokens_tt_something_else"));
    }

    #[test]
    fn import_tokens_internal_invalid_token_ident() {
        assert!(import_tokens_internal(quote!(let 3 * 2 = my_crate::something)).is_err());
    }

    #[test]
    fn import_tokens_internal_invalid_path() {
        assert!(import_tokens_internal(quote!(let my_tokens = 2 - 2)).is_err());
    }

    #[test]
    fn import_tokens_inner_internal_basic() {
        assert!(import_tokens_inner_internal(quote! {
            my_ident,
            fn my_function() -> u32 {
                33
            }
        })
        .unwrap()
        .to_string()
        .contains("my_ident"));
    }

    #[test]
    fn import_tokens_inner_internal_impl() {
        assert!(import_tokens_inner_internal(quote! {
            another_ident,
            impl Something for MyThing {
                fn something() -> CoolStuff {
                    CoolStuff {}
                }
            }
        })
        .unwrap()
        .to_string()
        .contains("something ()"));
    }

    #[test]
    fn import_tokens_inner_internal_missing_comma() {
        assert!(import_tokens_inner_internal(quote! {
            {
                another_ident
                impl Something for MyThing {
                    fn something() -> CoolStuff {
                        CoolStuff {}
                    }
                }
            }
        })
        .is_err());
    }

    #[test]
    fn import_tokens_inner_internal_non_item() {
        assert!(import_tokens_inner_internal(quote! {
            {
                another_ident,
                2 + 2
            }
        })
        .is_err());
    }

    #[test]
    fn test_parse_use_stmt() {
        assert!(use_internal(
            quote!(),
            quote!(
                use some::path;
            ),
            ProcMacroType::Attribute,
        )
        .is_ok());
        assert!(use_internal(
            quote!(),
            quote!(
                use some::path
            ),
            ProcMacroType::Normal,
        )
        .is_err());
        assert!(use_internal(
            quote!(),
            quote!(
                use some::
            ),
            ProcMacroType::Attribute,
        )
        .is_err());
        assert!(use_internal(
            quote!(),
            quote!(
                pub use some::long::path;
            ),
            ProcMacroType::Attribute,
        )
        .is_ok());
    }

    #[test]
    fn test_snake_case() {
        assert_eq!(to_snake_case("ThisIsATriumph"), "this_is_a_triumph");
        assert_eq!(
            to_snake_case("IAmMakingANoteHere"),
            "i_am_making_a_note_here"
        );
        assert_eq!(to_snake_case("huge_success"), "huge_success");
        assert_eq!(
            to_snake_case("It's hard to   Overstate my satisfaction!!!"),
            "its_hard_to_overstate_my_satisfaction"
        );
        assert_eq!(
            to_snake_case("__aperature_science__"),
            "__aperature_science__"
        );
        assert_eq!(
            to_snake_case("WeDoWhatWeMustBecause!<We, Can>()"),
            "we_do_what_we_must_because_we_can"
        );
        assert_eq!(
            to_snake_case("For_The_Good_of_all_of_us_Except_TheOnes_Who Are Dead".to_string()),
            "for_the_good_of_all_of_us_except_the_ones_who_are_dead"
        );
        assert_eq!(to_snake_case("".to_string()), "");
    }
}
