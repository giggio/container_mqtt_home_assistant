extern crate proc_macro;
extern crate quote;
extern crate syn;
use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[proc_macro_derive(ComponentDetailsGetter)]
pub fn derive_component_details_getter(input: TokenStream) -> TokenStream {
    let derive_input = parse_macro_input!(input as DeriveInput);
    let name = derive_input.ident;
    let generated = quote! {
        impl ComponentDetailsGetter for #name {
            fn details(&self) -> &ComponentDetails {
                &self.details
            }
        }
    };
    generated.into()
}
