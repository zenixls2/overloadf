#![feature(proc_macro_diagnostic)]
extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate quote;
use lazy_static::lazy_static;
use proc_macro::TokenStream;
use quote::ToTokens;
use std::collections::HashSet;
use std::sync::Mutex;
use syn::spanned::Spanned;

lazy_static! {
    static ref NAMINGS: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

#[proc_macro_attribute]
pub fn overload(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(attr as syn::AttributeArgs);
    // not used, currently specialization on Trait parameter is not well defined
    let _default = if !args.is_empty() {
        if args[0].clone().into_token_stream().to_string() == "default" {
            quote!(default)
        } else {
            quote!()
        }
    } else {
        quote!()
    };
    let ast = syn::parse_macro_input!(item as syn::ItemFn);
    let span = ast.span().unstable();
    let generics = ast.sig.generics;
    let attrs = ast.attrs;
    let vis = ast.vis;
    let constness = ast.sig.constness;
    let unsafety = ast.sig.unsafety;
    let asyncness = ast.sig.asyncness;
    let ident = ast.sig.ident.clone();
    let inputs = ast.sig.inputs;
    let mut output = match ast.sig.output {
        x @ syn::ReturnType::Default => quote!(#x),
        syn::ReturnType::Type(_, t) => quote!(#t),
    };
    let shared_type = format_ident!("Overloader_{}", ident);
    let mut input_types = vec![];
    let mut input_params = vec![];
    let mut param_assign = vec![];
    for (i, tp) in inputs.iter().enumerate() {
        if let syn::FnArg::Typed(tp) = tp {
            let pat: syn::Pat = Box::leak(tp.pat.clone()).clone();
            let ty: syn::Type = Box::leak(tp.ty.clone()).clone();
            input_params.push(format_ident!("_{}", i));
            input_types.push(ty);
            param_assign.push(pat);
        }
    }
    let body = ast.block.stmts;
    let not_defined = NAMINGS.lock().unwrap().insert(ast.sig.ident.to_string());
    let prepare = if not_defined {
        quote!(
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            #[allow(dead_code)]
            #[derive(Copy, Clone)]
            #vis struct #shared_type;
            unsafe impl Send for #shared_type {}
            unsafe impl Sync for #shared_type {}
            #[allow(non_upper_case_globals)]
            #vis static #ident: #shared_type = #shared_type;
        )
    } else {
        quote!()
    };
    let result;
    let block;
    if constness.is_some() {
        span.warning(
            "const fn is not supported. ".to_owned()
                + "Will ignore const to produce workable functions",
        )
        .emit();
    }
    if unsafety.is_some() {
        span.warning(
            "unsafe fn is not supported. ".to_owned()
                + "Will wrap in a unsafe block to make function safe.",
        )
        .emit();
        if asyncness.is_some() {
            output = quote!(std::pin::Pin<Box<dyn std::future::Future<Output = #output>>>);
            block = quote!(
                let (#(#param_assign),*,) = args;
                unsafe {
                    Box::pin(async move {
                        #(#body)*
                    })
                }
            );
        } else {
            block = quote!(
                let (#(#param_assign),*,) = args;
                unsafe {
                    #(#body)*
                }
            );
        }
    /*result = quote!(
        #prepare
        impl std::ops::Deref for #shared_type
        {
            type Target = unsafe fn(#(#input_types),*) -> #output;
            fn deref(&self) -> &Self::Target {
                union C {
                    a: (),
                    b: std::mem::ManuallyDrop<unsafe fn(#(#input_types),*) -> #output>,
                }
                static mut P: C = C {a: ()};
                unsafe fn u#generics(#inputs) -> #output {
                    #(#body)*
                }
                unsafe {
                    P.b = std::mem::ManuallyDrop::new(u);
                    P.b.deref()
                }
            }
        }
    );*/
    } else {
        if asyncness.is_some() {
            output = quote!(std::pin::Pin<Box<dyn std::future::Future<Output = #output>>>);
            block = quote!(
                let (#(#param_assign),*,) = args;
                Box::pin(async move {
                    #(#body)*
                })
            );
        } else {
            block = quote!(
                let (#(#param_assign),*,) = args;
                #(#body)*
            );
        }
    }
    result = quote!(
        #prepare
        impl#generics std::ops::FnOnce<(#(#input_types),*,)> for #shared_type {
            type Output = #output;
            #(#attrs)*
            #[inline]
            extern "rust-call" fn call_once(self, args: (#(#input_types),*,)) -> Self::Output {
                #block
            }
        }
        impl#generics std::ops::FnMut<(#(#input_types),*,)> for #shared_type {
            #(#attrs)*
            #[inline]
            extern "rust-call" fn call_mut(&mut self, args: (#(#input_types),*,)) -> Self::Output {
                #block
            }
        }
        impl#generics std::ops::Fn<(#(#input_types),*,)> for #shared_type {
            #(#attrs)*
            #[inline]
            extern "rust-call" fn call(&self, args: (#(#input_types),*,)) -> Self::Output {
                #block
            }
        }
    );
    result.into()
}
