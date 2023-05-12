#![feature(proc_macro_diagnostic, proc_macro_span)]
extern crate proc_macro;
#[macro_use]
extern crate syn;
#[macro_use]
extern crate quote;
use core::cmp::Ordering;
use lazy_static::lazy_static;
use proc_macro::TokenStream;
use quote::ToTokens;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use syn::spanned::Spanned;
mod fn_struct;
mod input_iter;

lazy_static! {
    static ref NAMINGS: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
    static ref TRAIT_IDENTS: Mutex<HashMap<String, Vec<String>>> = Mutex::new(HashMap::new());
    static ref DEFAULT_DEFINITION: Mutex<HashMap<
        String, HashMap<String, String> // TraitItemMethod
        >> = Mutex::new(HashMap::new());
}

macro_rules! quotation_expand {
    ($x: tt) => {
        if $x.is_empty() {
            quote!(())
        } else {
            quote!((#(#$x),*,))
        }
    }
}

macro_rules! fn_impl {
    (
        $impl_generics: tt,
        $input_types: tt,
        $shared_type: tt,
        $where_clause: tt,
        $output: tt,
        $attrs: tt,
        $block: tt
    ) => {
        quote!(
            impl #$impl_generics core::ops::FnOnce<#$input_types> for #$shared_type #$where_clause {
                type Output = #$output;
                #(#$attrs)*
                #[inline]
                extern "rust-call" fn call_once(self, args: #$input_types) -> Self::Output {
                    #$block
                }
            }
            impl #$impl_generics core::ops::FnMut<#$input_types> for #$shared_type #$where_clause {
                #(#$attrs)*
                #[inline]
                extern "rust-call" fn call_mut(&mut self, args: #$input_types) -> Self::Output {
                    #$block
                }
            }
            impl #$impl_generics core::ops::Fn<#$input_types> for #$shared_type #$where_clause {
                #(#$attrs)*
                #[inline]
                extern "rust-call" fn call(&self, args: #$input_types) -> Self::Output {
                    #$block
                }
            }
        )
    };
    (
        $impl_generics: tt,
        $input_types: tt,
        $shared_type: tt,
        $where_clause: tt,
        $output: tt,
        $attrs: tt,
        $block: tt,
        $tp: tt
    ) => {
        {
            let shared_type = quote!(#$shared_type<#$tp>);
            fn_impl!(
                $impl_generics,
                $input_types,
                shared_type,
                $where_clause,
                $output,
                $attrs,
                $block
            )
        }
    }
}

fn process_trait(mut item: syn::ItemTrait) -> TokenStream {
    let ident = item.ident.to_string();
    let mut map: HashMap<String, Vec<syn::TraitItemMethod>> = HashMap::new();
    let mut fn_map: HashMap<String, syn::TraitItemMethod> = HashMap::new();
    let mut items = vec![];
    for i in &item.items {
        let t = i.clone();
        if let syn::TraitItem::Method(item_method) = t {
            let method_sig = sig_normalize(&item_method.sig);
            map.entry(item_method.sig.ident.to_string())
                .and_modify(|e| e.push(item_method.clone()))
                .or_insert_with(|| vec![item_method.clone()]);
            fn_map.insert(method_sig, item_method);
        } else {
            items.push(t);
        }
    }
    let mut shared_fields = vec![];
    let mut prepares = vec![];
    for (s, i) in map.iter() {
        match i.len().cmp(&1) {
            Ordering::Greater => {
                let const_field = format_ident!("{}", s);
                shared_fields.push(const_field.to_string());
                let shared_type = format_ident!("Overloader_{}_{}", ident, s);
                let const_stream: TokenStream = quote!(
                    #[allow(non_upper_case_globals)]
                    const #const_field: #shared_type<Self> = #shared_type::<Self>(core::marker::PhantomData);
                ).into();
                let t = syn::parse_macro_input!(const_stream as syn::TraitItemConst);
                items.push(syn::TraitItem::Const(t));
                let vis = item.vis.clone();
                let prepare = quote!(
                    #[doc(hidden)]
                    #[allow(non_camel_case_types)]
                    #[allow(dead_code)]
                    #[derive(Copy, Clone)]
                    #vis struct #shared_type<S>(core::marker::PhantomData<S>);
                    unsafe impl<S> Send for #shared_type<S> {}
                    unsafe impl<S> Sync for #shared_type<S> {}
                );
                prepares.push(prepare);
            }
            Ordering::Equal => {
                items.push(syn::TraitItem::Method(i[0].clone()));
            }
            Ordering::Less => {}
        }
    }
    DEFAULT_DEFINITION.lock().unwrap().insert(
        ident.clone(),
        fn_map
            .into_iter()
            .map(|(k, v)| (k, v.into_token_stream().to_string()))
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
        .replace("Self", &to.to_token_stream().to_string())
        .replace("self", "__self");
    syn::parse_str(&input_str)
}

fn get_output(ast: &syn::ReturnType, tp: &syn::Type) -> proc_macro2::TokenStream {
    let new_output: syn::ReturnType = replace_self(&ast, tp).unwrap();
    match new_output {
        syn::ReturnType::Default => quote!(()),
        syn::ReturnType::Type(_, t) => quote!(#t),
    }
}

struct IdentGen(pub u32);
impl IdentGen {
    #[inline]
    pub fn new() -> Self {
        Self(0)
    }
    #[inline]
    pub fn ident(&mut self) -> syn::Ident {
        self.0 += 1;
        syn::Ident::new(&format!("_{}", self.0), proc_macro2::Span::call_site())
    }
    #[inline]
    pub fn id(&self, id: u32) -> syn::Ident {
        syn::Ident::new(&format!("_{}", id), proc_macro2::Span::call_site())
    }
}

fn generics_normalize(ast: &syn::Generics) -> syn::Generics {
    let mut gen = ast.clone();
    let mut matching_table = HashMap::new();
    let mut idgen = IdentGen::new();
    for gen_param in gen.params.iter_mut() {
        match gen_param {
            syn::GenericParam::Type(t) => {
                matching_table
                    .entry(t.ident.to_string())
                    .and_modify(|e| {
                        t.ident = idgen.id(*e);
                    })
                    .or_insert_with(|| {
                        t.ident = idgen.ident();
                        idgen.0
                    });
            }
            syn::GenericParam::Lifetime(l) => {
                matching_table
                    .entry(l.lifetime.ident.to_string())
                    .and_modify(|e| {
                        l.lifetime.ident = idgen.id(*e);
                    })
                    .or_insert_with(|| {
                        l.lifetime.ident = idgen.ident();
                        idgen.0
                    });
                for ll in l.bounds.iter_mut() {
                    matching_table
                        .entry(ll.ident.to_string())
                        .and_modify(|e| {
                            ll.ident = idgen.id(*e);
                        })
                        .or_insert_with(|| {
                            ll.ident = idgen.ident();
                            idgen.0
                        });
                }
            }
            syn::GenericParam::Const(_) => {
                // left untouched, should not have any renaming here
            }
        }
    }
    if let Some(mut wh) = gen.where_clause.clone() {
        let mut p = syn::punctuated::Punctuated::<_, syn::Token![,]>::new();
        for predicate in wh.predicates.iter_mut() {
            let mut take = true;
            match predicate {
                syn::WherePredicate::Type(t) => {
                    if let Some(l) = &mut t.lifetimes {
                        for ll in l.lifetimes.iter_mut() {
                            if let Some(v) = matching_table.get(&ll.lifetime.ident.to_string()) {
                                ll.lifetime.ident = idgen.id(*v);
                                for lll in ll.bounds.iter_mut() {
                                    if let Some(v) = matching_table.get(&lll.ident.to_string()) {
                                        lll.ident = idgen.id(*v);
                                    } else {
                                        take = false;
                                    }
                                }
                            } else {
                                take = false;
                            }
                        }
                    }
                    let ty = t
                        .bounded_ty
                        .to_token_stream()
                        .into_iter()
                        .map(|t| {
                            if let proc_macro2::TokenTree::Ident(id) = t.clone() {
                                if let Some(v) = matching_table.get(&id.to_string()) {
                                    return proc_macro2::TokenTree::Ident(proc_macro2::Ident::new(
                                        &format!("_{}", v),
                                        proc_macro2::Span::call_site(),
                                    ));
                                } else {
                                    take = false;
                                }
                            }
                            t
                        })
                        .collect::<proc_macro2::TokenStream>();
                    t.bounded_ty = syn::parse_str(&ty.to_string()).unwrap();
                }
                syn::WherePredicate::Lifetime(l) => {
                    if let Some(v) = matching_table.get(&l.lifetime.ident.to_string()) {
                        l.lifetime.ident = idgen.id(*v);
                        for ll in l.bounds.iter_mut() {
                            if let Some(v) = matching_table.get(&ll.ident.to_string()) {
                                ll.ident = idgen.id(*v);
                            } else {
                                // maybe too strict ?
                                take = false;
                            }
                        }
                    } else {
                        take = false;
                    }
                }
                syn::WherePredicate::Eq(_) => {} // unsupported
            }
            if take {
                p.push(predicate.clone());
            }
        }
        wh.predicates = p;
        gen.where_clause = Some(wh);
    }
    gen
}

fn sig_normalize(sig: &syn::Signature) -> String {
    let mut sig = sig.clone();
    let underscore_token: syn::token::Underscore = syn::parse_str("_").unwrap();
    for i in sig.inputs.iter_mut() {
        if let syn::FnArg::Typed(pt) = i {
            pt.pat = Box::new(syn::Pat::Wild(syn::PatWild {
                attrs: vec![],
                underscore_token,
            }));
        }
    }
    // not going to work for all cases, but should be enough
    sig.generics = generics_normalize(&sig.generics);
    // rust doesn't differentiate functions by their output
    // neither could fn traits do
    sig.output = syn::ReturnType::Default;
    sig.into_token_stream().to_string()
}

fn impl_method_to_non_trait(
    tp: &syn::Type,
    ast: &syn::ImplItemMethod,
) -> proc_macro2::TokenStream {
    let span = ast.span().unstable();
    let (impl_generics, _ty_generics, where_clause) = &ast.sig.generics.split_for_impl();
    let attrs = &ast.attrs;
    let unsafety = &ast.sig.unsafety;
    let asyncness = &ast.sig.asyncness;
    let ident = ast.sig.ident.clone().into_token_stream().to_string();
    let tp_str = tp.into_token_stream().to_string().replace(" ", "_");
    let shared_type = format_ident!("Overloader_{}_{}", tp_str, ident);
    let inputs = &ast.sig.inputs;
    let mut output = get_output(&ast.sig.output, tp);
    let mut input_types = Vec::<syn::Type>::new();
    let mut input_params = vec![];
    let mut param_assign = Vec::<syn::Pat>::new();
    for (i, itp) in inputs.iter().enumerate() {
        input_params.push(format_ident!("_{}", i));
        match itp {
            syn::FnArg::Typed(itp) => {
                input_types.push(Box::leak(itp.ty.clone()).clone());
                param_assign.push(Box::leak(itp.pat.clone()).clone());
            }
            syn::FnArg::Receiver(r) => {
                let ty = match (r.reference.as_ref(), r.mutability.as_ref()) {
                    (Some(_), Some(_)) => quote!(&mut #tp),
                    (Some(_), None) => quote!(&#tp),
                    (None, Some(_)) => quote!(mut #tp),
                    (None, None) => quote!(#tp),
                };
                input_types.push(syn::parse_str(&ty.to_string()).unwrap());
                param_assign.push(syn::parse_str("__self").unwrap());
            }
        }
    }
    let new_block: syn::Block = replace_self(&ast.block, tp).unwrap();
    let body = &new_block.stmts;
    let param_assign = quotation_expand!(param_assign);
    let input_types = quotation_expand!(input_types);
    let block;
    if unsafety.is_some() {
        span.warning(
            "unsafe fn is not supported. ".to_owned()
                + "Will wrap in a unsafe block to make function safe.",
        )
        .emit();
        if asyncness.is_some() {
            output = quote!(core::pin::Pin<Box<dyn core::future::Future<Output = #output>>>);
            block = quote!(
                let #param_assign = args;
                unsafe {
                    Box::pin(async move {
                        #(#body)*
                    })
                }
            );
        } else {
            block = quote!(
                let #param_assign = args;
                unsafe {
                    #(#body)*
                }
            );
        }
    } else {
        if asyncness.is_some() {
            output = quote!(core::pin::Pin<Box<dyn core::future::Future<Output = #output>>>);
            block = quote!(
                let #param_assign = args;
                Box::pin(async move {
                    #(#body)*
                })
            );
        } else {
            block = quote!(
                let #param_assign = args;
                #(#body)*
            );
        }
    }
    let result = fn_impl!(
        impl_generics,
        input_types,
        shared_type,
        where_clause,
        output,
        attrs,
        block
    );
    result
}

fn trait_method_to_fn_trait(
    trait_path: &syn::Path,
    tt: &syn::Ident,
    tp: &syn::Type,
    ast: &syn::TraitItemMethod,
) -> proc_macro2::TokenStream {
    let span = ast.span().unstable();
    if let Some(block) = &ast.default {
        let (impl_generics, _ty_generics, where_clause) = ast.sig.generics.split_for_impl();
        let attrs = &ast.attrs;
        let unsafety = &ast.sig.unsafety;
        let asyncness = &ast.sig.asyncness;
        let ident = ast.sig.ident.clone();
        let shared_type = format_ident!("Overloader_{}_{}", tt, ident);
        let inputs = &ast.sig.inputs;
        let mut output = get_output(&ast.sig.output, tp);
        let mut input_types = Vec::<syn::Type>::new();
        let mut input_params = vec![];
        let mut param_assign = Vec::<syn::Pat>::new();
        for (i, itp) in inputs.iter().enumerate() {
            input_params.push(format_ident!("_{}", i));
            match itp {
                syn::FnArg::Typed(itp) => {
                    let ty: syn::Type = Box::leak(itp.ty.clone()).clone();
                    let ty = if let syn::Type::Path(path) = ty.clone() {
                        if path.path.segments.len() == 1 {
                            replace_self(ty, trait_path).unwrap()
                        } else {
                            replace_self(ty, quote!(<#tp as #trait_path>)).unwrap()
                        }
                    } else {
                        ty
                    };
                    input_types.push(ty);
                    param_assign.push(Box::leak(itp.pat.clone()).clone());
                }
                syn::FnArg::Receiver(r) => {
                    let ty = match (r.reference.as_ref(), r.mutability.as_ref()) {
                        (Some(_), Some(_)) => quote!(&mut #tp),
                        (Some(_), None) => quote!(&#tp),
                        (None, Some(_)) => quote!(mut #tp),
                        (None, None) => quote!(#tp),
                    };
                    input_types.push(syn::parse_str(&ty.to_string()).unwrap());
                    param_assign.push(syn::parse_str("__self").unwrap());
                }
            }
        }
        let new_block: syn::Block = replace_self(block, tp).unwrap();
        let body = &new_block.stmts;
        let param_assign = quotation_expand!(param_assign);
        let input_types = quotation_expand!(input_types);
        let block;
        if unsafety.is_some() {
            span.warning(
                "unsafe fn is not supported. ".to_owned()
                    + "Will wrap in a unsafe block to make function safe.",
            )
            .emit();
            if asyncness.is_some() {
                output = quote!(core::pin::Pin<Box<dyn core::future::Future<Output = #output>>>);
                block = quote!(
                    let #param_assign = args;
                    unsafe {
                        Box::pin(async move {
                            #(#body)*
                        })
                    }
                );
            } else {
                block = quote!(
                    let #param_assign = args;
                    unsafe {
                        #(#body)*
                    }
                );
            }
        } else {
            if asyncness.is_some() {
                output = quote!(core::pin::Pin<Box<dyn core::future::Future<Output = #output>>>);
                block = quote!(
                    let #param_assign = args;
                    Box::pin(async move {
                        #(#body)*
                    })
                );
            } else {
                block = quote!(
                    let #param_assign = args;
                    #(#body)*
                );
            }
        }
        let result = fn_impl!(
            impl_generics,
            input_types,
            shared_type,
            where_clause,
            output,
            attrs,
            block,
            tp
        );
        return result;
    } else {
        span.error(format!(
            "trait function {} with empty default",
            ast.into_token_stream().to_string()
        ))
        .emit();
    }
    quote!()
}

fn impl_method_to_fn_trait(
    trait_path: &syn::Path,
    tt: &syn::Ident,
    tp: &syn::Type,
    ast: &syn::ImplItemMethod,
) -> proc_macro2::TokenStream {
    let span = ast.span().unstable();
    let (impl_generics, _ty_generics, where_clause) = ast.sig.generics.split_for_impl();
    let attrs = &ast.attrs;
    let unsafety = &ast.sig.unsafety;
    let asyncness = &ast.sig.asyncness;
    let ident = ast.sig.ident.clone();
    let shared_type = format_ident!("Overloader_{}_{}", tt, ident);
    let inputs = &ast.sig.inputs;
    let mut output = get_output(&ast.sig.output, tp);
    let mut input_types = vec![];
    let mut input_params = vec![];
    let mut param_assign = vec![];
    for (i, itp) in inputs.iter().enumerate() {
        match itp {
            syn::FnArg::Typed(itp) => {
                let pat: syn::Pat = Box::leak(itp.pat.clone()).clone();
                let ty: syn::Type = Box::leak(itp.ty.clone()).clone();
                let ty = if let syn::Type::Path(path) = ty.clone() {
                    if path.path.segments.len() == 1 {
                        replace_self(ty, trait_path).unwrap()
                    } else {
                        replace_self(ty, quote!(<#tp as #trait_path>)).unwrap()
                    }
                } else {
                    ty
                };
                input_params.push(format_ident!("_{}", i));
                input_types.push(ty);
                param_assign.push(pat);
            }
            syn::FnArg::Receiver(r) => {
                input_params.push(format_ident!("_{}", i));
                let ty = match (r.reference.as_ref(), r.mutability.as_ref()) {
                    (Some(_), Some(_)) => quote!(&mut #tp),
                    (Some(_), None) => quote!(&#tp),
                    (None, Some(_)) => quote!(mut #tp),
                    (None, None) => quote!(#tp),
                };
                let ty: syn::Type = syn::parse_str(&ty.to_string()).unwrap();
                input_types.push(ty);
                let pat: syn::Pat = syn::parse_str("__self").unwrap();
                param_assign.push(pat);
            }
        }
    }
    let new_block: syn::Block = replace_self(&ast.block, tp).unwrap();
    let body = &new_block.stmts;
    let param_assign = quotation_expand!(param_assign);
    let input_types = quotation_expand!(input_types);
    let block;
    if unsafety.is_some() {
        span.warning(
            "unsafe fn is not supported. ".to_owned()
                + "Will wrap in a unsafe block to make function safe.",
        )
        .emit();
        if asyncness.is_some() {
            output = quote!(core::pin::Pin<Box<dyn core::future::Future<Output = #output>>>);
            block = quote!(
                let #param_assign = args;
                unsafe {
                    Box::pin(async move {
                        #(#body)*
                    })
                }
            );
        } else {
            block = quote!(
                let #param_assign = args;
                unsafe {
                    #(#body)*
                }
            );
        }
    } else {
        if asyncness.is_some() {
            output = quote!(core::pin::Pin<Box<dyn core::future::Future<Output = #output>>>);
            block = quote!(
                let #param_assign = args;
                Box::pin(async move {
                    #(#body)*
                })
            );
        } else {
            block = quote!(
                let #param_assign = args;
                #(#body)*
            );
        }
    }
    fn_impl!(
        impl_generics,
        input_types,
        shared_type,
        where_clause,
        output,
        attrs,
        block,
        tp
    )
}

fn process_impl(mut item: syn::ItemImpl) -> TokenStream {
    let self_type = Box::leak(item.self_ty.clone());
    let span = item.span().unstable();
    let mut generated = vec![];
    let mut items = vec![];
    if let Some((_, path, _)) = item.trait_.clone() {
        // impl Trait for Struct {}
        if let Some(pathseg) = path.segments.first() {
            let ident = &pathseg.ident;
            if let Some(shared_fields) = TRAIT_IDENTS.lock().unwrap().get(&ident.to_string()) {
                let mut set = HashSet::new();
                for i in &item.items {
                    if let syn::ImplItem::Method(item_method) = i {
                        let method_id = item_method.sig.ident.to_string();
                        let method_sig = sig_normalize(&item_method.sig);
                        set.insert(method_sig);
                        if shared_fields.iter().any(|e| e == &method_id) {
                            generated.push(impl_method_to_fn_trait(
                                &path,
                                ident,
                                &self_type,
                                item_method,
                            ));
                        } else {
                            items.push(syn::ImplItem::Method(item_method.clone()));
                        }
                    } else {
                        items.push(i.clone());
                    }
                }
                // all trait function definitions
                if let Some(map) = DEFAULT_DEFINITION.lock().unwrap().get(&ident.to_string()) {
                    for (k, v) in map.iter() {
                        // check if is not implemented
                        if !set.contains(k) {
                            let item_method: syn::TraitItemMethod = syn::parse_str(v).unwrap();
                            let method_id = item_method.sig.ident.to_string();
                            // check if method needs overloading
                            if shared_fields.iter().any(|e| e == &method_id) {
                                generated.push(trait_method_to_fn_trait(
                                    &path,
                                    ident,
                                    &self_type,
                                    &item_method,
                                ));
                            }
                        }
                    }
                } else {
                    span.error("definition of trait methods not found").emit();
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
                        let tp_str = self_type.into_token_stream().to_string().replace(" ", "_");
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

fn param_variants(
    input_types: Vec<syn::Type>,
    param_assign: Vec<syn::Pat>,
    default_values: Vec<Option<fn_struct::Assign>>,
) -> input_iter::InputIter {
    input_iter::InputIter::new(input_types, param_assign, default_values)
}

fn process_fn(ast: syn::ItemFn) -> TokenStream {
    let span = ast.span().unstable();
    let (impl_generics, _ty_generics, where_clause) = ast.sig.generics.split_for_impl();
    let attrs = ast.attrs;
    let vis = ast.vis;
    let constness = ast.sig.constness;
    let unsafety = ast.sig.unsafety;
    let asyncness = ast.sig.asyncness;
    let ident = ast.sig.ident.clone();
    let inputs = ast.sig.inputs;
    let mut output = match ast.sig.output {
        syn::ReturnType::Default => quote!(()),
        syn::ReturnType::Type(_, t) => quote!(#t),
    };
    let shared_type = format_ident!("Overloader_{}", ident);
    let mut default_values = vec![];
    let mut input_types = vec![];
    let mut param_assign = vec![];
    for tp in inputs.iter() {
        if let syn::FnArg::Typed(tp) = tp {
            let mut assign: Option<fn_struct::Assign> = None;
            for attr in &tp.attrs {
                if attr.path.is_ident("default") {
                    assign = Some(attr.parse_args().unwrap());
                    break;
                }
            }
            let pat: syn::Pat = Box::leak(tp.pat.clone()).clone();
            let ty: syn::Type = Box::leak(tp.ty.clone()).clone();
            input_types.push(ty);
            param_assign.push(pat);
            default_values.push(assign);
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
    let mut results = vec![];
    if constness.is_some() {
        span.warning(
            "const fn is not supported. ".to_owned()
                + "Will ignore const to produce workable functions",
        )
        .emit();
    }
    let param_iter = param_variants(input_types, param_assign, default_values);
    for (input_types, param_assign, defaults) in param_iter {
        let input_types = quotation_expand!(input_types);
        let param_assign = quotation_expand!(param_assign);
        let block;
        if unsafety.is_some() {
            span.warning(
                "unsafe fn is not supported. ".to_owned()
                    + "Will wrap in a unsafe block to make function safe.",
            )
            .emit();
            if asyncness.is_some() {
                output = quote!(core::pin::Pin<Box<dyn core::future::Future<Output = #output>>>);
                block = quote!(
                    let #param_assign = args;
                    #(#defaults)*
                    unsafe {
                        Box::pin(async move {
                            #(#body)*
                        })
                    }
                );
            } else {
                block = quote!(
                    let #param_assign = args;
                    #(#defaults)*
                    unsafe {
                        #(#body)*
                    }
                );
            }
        } else {
            if asyncness.is_some() {
                output = quote!(core::pin::Pin<Box<dyn core::future::Future<Output = #output>>>);
                block = quote!(
                    let #param_assign = args;
                    #(#defaults)*
                    Box::pin(async move {
                        #(#body)*
                    })
                );
            } else {
                block = quote!(
                    let #param_assign = args;
                    #(#defaults)*
                    #(#body)*
                );
            }
        }
        let result = fn_impl!(
            impl_generics,
            input_types,
            shared_type,
            where_clause,
            output,
            attrs,
            block
        );
        results.push(result);
    }
    quote!(
        #prepare
        #(#results)*
    )
    .into()
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
