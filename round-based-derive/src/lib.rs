use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, Generics, Ident, Token, Variant, parse_macro_input};

#[proc_macro_derive(ProtocolMsg, attributes(protocol_msg))]
pub fn protocol_msg(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let mut root = None;

    for attr in input.attrs {
        if !attr.path.is_ident("protocol_msg") {
            continue;
        }
        if root.is_some() {
            return quote_spanned! { attr.path.span() => compile_error!("#[protocol_msg] attribute appears more than once"); }.into();
        }
        let tokens = attr.tokens.into();
        root = Some(parse_macro_input!(tokens as RootAttribute));
    }

    let root_path = root
        .map(|root| root.path)
        .unwrap_or_else(|| Punctuated::from_iter([Ident::new("round_based", Span::call_site())]));

    let enum_data = match input.data {
        Data::Enum(e) => e,
        Data::Struct(s) => {
            return quote_spanned! {s.struct_token.span => compile_error!("only enum may implement ProtocolMsg");}.into()
        }
        Data::Union(s) => {
            return quote_spanned! {s.union_token.span => compile_error!("only enum may implement ProtocolMsg");}.into()
        }
    };

    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let round_method_impl = if !enum_data.variants.is_empty() {
        round_method(&name, enum_data.variants.iter())
    } else {
        // Special case for empty enum. Empty protocol message is useless, but let it be
        quote! { match *self {} }
    };

    let impl_protocol_msg = quote! {
        impl #impl_generics #root_path::ProtocolMsg for #name #ty_generics #where_clause {
            fn round(&self) -> u16 {
                #round_method_impl
            }
        }
    };

    let impl_round_msg = round_msgs(
        &root_path,
        &name,
        &input.generics,
        enum_data.variants.iter(),
    );

    proc_macro::TokenStream::from(quote! {
        #impl_protocol_msg
        #impl_round_msg
    })
}

fn round_method<'v>(enum_name: &Ident, variants: impl Iterator<Item = &'v Variant>) -> TokenStream {
    let match_variants = (0u16..).zip(variants).map(|(i, variant)| {
        let variant_name = &variant.ident;
        match &variant.fields {
            Fields::Unit => quote_spanned! {
                variant.ident.span() =>
                #enum_name::#variant_name => compile_error!("unit variants are not allowed in ProtocolMsg"),
            },
            Fields::Named(_) => quote_spanned! {
                variant.ident.span() =>
                #enum_name::#variant_name{..} => compile_error!("named variants are not allowed in ProtocolMsg"),
            },
            Fields::Unnamed(unnamed) => if unnamed.unnamed.len() == 1 {
                quote_spanned! {
                    variant.ident.span() =>
                    #enum_name::#variant_name(_) => #i,
                }
            } else {
                quote_spanned! {
                    variant.ident.span() =>
                    #enum_name::#variant_name(..) => compile_error!("this variant must contain exactly one field to be valid ProtocolMsg"),
                }
            },
        }
    });
    quote! {
        match self {
            #(#match_variants)*
        }
    }
}

fn round_msgs<'v>(
    root_path: &RootPath,
    enum_name: &Ident,
    generics: &Generics,
    variants: impl Iterator<Item = &'v Variant>,
) -> TokenStream {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let impls = (0u16..).zip(variants).map(|(i, variant)| {
        let variant_name = &variant.ident;
        match &variant.fields {
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let msg_type = &unnamed.unnamed[0].ty;
                quote_spanned! {
                    variant.ident.span() =>
                    impl #impl_generics #root_path::RoundMsg<#msg_type> for #enum_name #ty_generics #where_clause {
                        const ROUND: u16 = #i;
                        fn to_protocol_msg(round_msg: #msg_type) -> Self {
                            #enum_name::#variant_name(round_msg)
                        }
                        fn from_protocol_msg(protocol_msg: Self) -> Result<#msg_type, Self> {
                            #[allow(unreachable_patterns)]
                            match protocol_msg {
                                #enum_name::#variant_name(msg) => Ok(msg),
                                _ => Err(protocol_msg),
                            }
                        }
                    }
                }
            }
            _ => quote! {},
        }
    });
    quote! {
        #(#impls)*
    }
}

type RootPath = Punctuated<Ident, Token![::]>;

#[allow(dead_code)]
struct RootAttribute {
    paren: syn::token::Paren,
    root: kw::root,
    eq: Token![=],
    path: RootPath,
}

impl Parse for RootAttribute {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        let paren = syn::parenthesized!(content in input);
        let root = content.parse::<kw::root>()?;
        let eq = content.parse::<Token![=]>()?;
        let path = RootPath::parse_separated_nonempty_with(&content, Ident::parse_any)?;
        let _ = content.parse::<syn::parse::Nothing>()?;

        Ok(Self {
            paren,
            root,
            eq,
            path,
        })
    }
}

mod kw {
    syn::custom_keyword! { root }
}
