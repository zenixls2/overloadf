#![feature(proc_macro_diagnostic)]
extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate quote;
use lazy_static::lazy_static;
use proc_macro::TokenStream;
use quote::ToTokens;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use syn::spanned::Spanned;

lazy_static! {
    static ref NAMINGS: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
    static ref TRAIT_IDENTS: Mutex<HashMap<String, Vec<String>>> = Mutex::new(HashMap::new());
    static ref DEFAULT_DEFINITION: Mutex<HashMap<
        String, HashMap<String, Vec<String>> // TraitItemMethod
        >> = Mutex::new(HashMap::new());
}

fn process_trait(mut item: syn::ItemTrait) -> TokenStream {
    let ident = item.ident.to_string();
    let mut map: HashMap<String, Vec<syn::TraitItemMethod>> = HashMap::new();
    let mut items = vec![];
    for i in &item.items {
        let t = i.clone();
        if let syn::TraitItem::Method(item_method) = t {
            map.entry(item_method.sig.ident.to_string())
                .and_modify(|e| {
                    e.push(item_method.clone());
                })
                .or_insert_with(|| vec![item_method]);
        } else {
            items.push(t);
        }
    }
    let mut shared_fields = vec![];
    let mut prepares = vec![];
    for (s, i) in map.iter() {
        if i.len() > 1 {
            let const_field = format_ident!("{}", s);
            shared_fields.push(const_field.to_string());
            let shared_type = format_ident!("Overloader_{}_{}", ident, s);
            let const_stream: TokenStream = quote!(
                const #const_field: #shared_type<Self> = #shared_type::<Self>(std::marker::PhantomData);
            ).into();
            let t = syn::parse_macro_input!(const_stream as syn::TraitItemConst);
            items.push(syn::TraitItem::Const(t));
            let vis = item.vis.clone();
            let prepare = quote!(
                #[doc(hidden)]
                #[allow(non_camel_case_types)]
                #[allow(dead_code)]
                #[derive(Copy, Clone)]
                #vis struct #shared_type<S>(std::marker::PhantomData<S>);
                unsafe impl<S> Send for #shared_type<S> {}
                unsafe impl<S> Sync for #shared_type<S> {}
            );
            prepares.push(prepare);
        } else if !i.is_empty() {
            items.push(syn::TraitItem::Method(i[0].clone()));
        }
    }
    DEFAULT_DEFINITION.lock().unwrap().insert(
        ident.clone(),
        map.into_iter()
            .map(|(k, v)| {
                (
                    k,
                    v.into_iter()
                        .map(|i| i.into_token_stream().to_string())
                        .collect(),
                )
            })
            .collect(),
    );
    TRAIT_IDENTS.lock().unwrap().insert(ident, shared_fields);
    item.items = items;
    let result = quote!(
        #(#prepares)*
        #item
    );
    result.into()
}

fn replace_self<F: ToTokens, T: ToTokens, O: syn::parse::Parse>(
    input: F,
    to: T,
) -> syn::parse::Result<O> {
    let input_str = input
        .to_token_stream()
        .to_string()
        .replace("Self", &to.to_token_stream().to_string());
    syn::parse_str(&input_str)
}

fn impl_method_to_non_trait(
    tp: &syn::Type,
    ast: &syn::ImplItemMethod,
) -> syn::export::TokenStream2 {
    let span = ast.span().unstable();
    let generics = &ast.sig.generics;
    let attrs = &ast.attrs;
    let unsafety = &ast.sig.unsafety;
    let asyncness = &ast.sig.asyncness;
    let ident = ast.sig.ident.clone().into_token_stream().to_string();
    let tp_str = tp.into_token_stream().to_string();
    let shared_type = format_ident!("Overloader_{}_{}", tp_str, ident);
    let inputs = &ast.sig.inputs;
    let new_output: syn::ReturnType = replace_self(&ast.sig.output, tp).unwrap();
    let mut output = match new_output {
        x @ syn::ReturnType::Default => quote!(#x),
        syn::ReturnType::Type(_, t) => quote!(#t),
    };
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
    let new_block: syn::Block = replace_self(&ast.block, tp).unwrap();
    let body = &new_block.stmts;
    let block;
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
    let result = quote!(
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
    result
}

fn impl_method_to_fn_trait(
    tt: &syn::Ident,
    tp: &syn::Type,
    ast: &syn::ImplItemMethod,
) -> syn::export::TokenStream2 {
    let span = ast.span().unstable();
    let generics = &ast.sig.generics;
    let attrs = &ast.attrs;
    let unsafety = &ast.sig.unsafety;
    let asyncness = &ast.sig.asyncness;
    let ident = ast.sig.ident.clone();
    let shared_type = format_ident!("Overloader_{}_{}", tt, ident);
    let inputs = &ast.sig.inputs;
    let new_output: syn::ReturnType = replace_self(&ast.sig.output, tp).unwrap();
    let mut output = match new_output {
        x @ syn::ReturnType::Default => quote!(#x),
        syn::ReturnType::Type(_, t) => quote!(#t),
    };
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
    let new_block: syn::Block = replace_self(&ast.block, tp).unwrap();
    let body = &new_block.stmts;
    let block;
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
    let result = quote!(
        impl#generics std::ops::FnOnce<(#(#input_types),*,)> for #shared_type<#tp> {
            type Output = #output;
            #(#attrs)*
            #[inline]
            extern "rust-call" fn call_once(self, args: (#(#input_types),*,)) -> Self::Output {
                #block
            }
        }
        impl#generics std::ops::FnMut<(#(#input_types),*,)> for #shared_type<#tp> {
            #(#attrs)*
            #[inline]
            extern "rust-call" fn call_mut(&mut self, args: (#(#input_types),*,)) -> Self::Output {
                #block
            }
        }
        impl#generics std::ops::Fn<(#(#input_types),*,)> for #shared_type<#tp> {
            #(#attrs)*
            #[inline]
            extern "rust-call" fn call(&self, args: (#(#input_types),*,)) -> Self::Output {
                #block
            }
        }
    );
    result
}

fn process_impl(mut item: syn::ItemImpl) -> TokenStream {
    let self_type = Box::leak(item.self_ty.clone());
    let span = item.span().unstable();
    let mut generated = vec![];
    let mut items = vec![];
    if let Some((_, path, _)) = item.trait_.clone() {
        // impl Trait for Struct {}
        if let Some(ident) = path.get_ident() {
            if let Some(shared_fields) = TRAIT_IDENTS.lock().unwrap().get(&ident.to_string()) {
                // TODO: default implementation
                for i in &item.items {
                    if let syn::ImplItem::Method(item_method) = i {
                        let method_id = item_method.sig.ident.to_string();
                        if shared_fields.iter().any(|e| e == &method_id) {
                            generated.push(impl_method_to_fn_trait(ident, &self_type, item_method));
                        } else {
                            items.push(syn::ImplItem::Method(item_method.clone()));
                        }
                    } else {
                        items.push(i.clone());
                    }
                }
            } else {
                span.error("definition of trait not found").emit();
            }
        } else {
            span.error("complex trait path (including colon) is not yet supported.")
                .emit();
        }
    } else {
        // normal impl Struct {}
        let mut fn_names = HashSet::new();
        let mut dup = HashSet::new();
        let mut undefined = HashSet::new();
        for i in &item.items {
            if let syn::ImplItem::Method(item_method) = i {
                let method_id = item_method.sig.ident.to_string();
                if !fn_names.insert(method_id.clone()) {
                    dup.insert(method_id);
                }
            }
        }
        for i in &item.items {
            if let syn::ImplItem::Method(item_method) = i {
                let vis = &item_method.vis;
                let method_id = item_method.sig.ident.to_string();
                if dup.get(&method_id).is_some() {
                    if undefined.insert(method_id.clone()) {
                        let const_field = &item_method.sig.ident;
                        let tp_str = self_type.into_token_stream().to_string();
                        let shared_type = format_ident!("Overloader_{}_{}", tp_str, method_id);
                        let const_stream: TokenStream = quote!(
                            #[allow(non_upper_case_globals)]
                            const #const_field: #shared_type = #shared_type;
                        )
                        .into();
                        let t = syn::parse_macro_input!(const_stream as syn::ImplItemConst);
                        items.push(syn::ImplItem::Const(t));
                        generated.push(quote!(
                            #[allow(non_camel_case_types, dead_code)]
                            #[derive(Copy, Clone)]
                            #vis struct #shared_type;
                            unsafe impl Send for #shared_type {}
                            unsafe impl Sync for #shared_type {}
                        ));
                    }
                    generated.push(impl_method_to_non_trait(&self_type, item_method));
                } else {
                    items.push(syn::ImplItem::Method(item_method.clone()));
                }
            } else {
                items.push(i.clone());
            }
        }
    }
    item.items = items;
    let result = quote!(
        #(#generated)*
        #item
    );
    result.into()
}

fn process_fn(ast: syn::ItemFn) -> TokenStream {
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
    if let Ok(ast) = syn::parse_macro_input::parse::<syn::ItemTrait>(item.clone()) {
        return process_trait(ast);
    } else if let Ok(ast) = syn::parse_macro_input::parse::<syn::ItemImpl>(item.clone()) {
        return process_impl(ast);
    } else if let Ok(ast) = syn::parse_macro_input::parse::<syn::ItemFn>(item.clone()) {
        return process_fn(ast);
    } else {
        for tree in item.into_iter() {
            tree.span()
                .warning("overload is only applicable to trait, impl, and function")
                .emit();
        }
    }
    quote!().into()
}
