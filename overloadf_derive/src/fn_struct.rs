use quote::{ToTokens, TokenStreamExt};
use std::iter::FromIterator;
use syn::parse::{discouraged::Speculative, Parse, ParseStream, Result};
use syn::punctuated::{Pair, Punctuated};
trait FilterAttrs<'a> {
    type Ret: Iterator<Item = &'a syn::Attribute>;
    fn outer(self) -> Self::Ret;
    fn inner(self) -> Self::Ret;
}

impl<'a, T> FilterAttrs<'a> for T
where
    T: IntoIterator<Item = &'a syn::Attribute>,
{
    type Ret = std::iter::Filter<T::IntoIter, fn(&&syn::Attribute) -> bool>;

    fn outer(self) -> Self::Ret {
        fn is_outer(attr: &&syn::Attribute) -> bool {
            match attr.style {
                syn::AttrStyle::Outer => true,
                _ => false,
            }
        }
        self.into_iter().filter(is_outer)
    }

    fn inner(self) -> Self::Ret {
        fn is_inner(attr: &&syn::Attribute) -> bool {
            match attr.style {
                syn::AttrStyle::Inner(_) => true,
                _ => false,
            }
        }
        self.into_iter().filter(is_inner)
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct Assign {
    pub eq_token: syn::token::Eq,
    pub right: Box<syn::Expr>,
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct PatType {
    pub attrs: Vec<syn::Attribute>,
    pub pat: Box<syn::Pat>,
    pub colon_token: Token![:],
    pub ty: Box<syn::Type>,
    pub assign: Option<Assign>,
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub enum FnArg {
    Receiver(syn::Receiver),
    Typed(PatType),
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct Signature {
    pub constness: Option<Token![const]>,
    pub asyncness: Option<Token![async]>,
    pub unsafety: Option<Token![unsafe]>,
    pub abi: Option<syn::Abi>,
    pub fn_token: Token![fn],
    pub ident: syn::Ident,
    pub generics: syn::Generics,
    pub paren_token: syn::token::Paren,
    pub inputs: Punctuated<FnArg, Token![,]>,
    pub variadic: Option<syn::Variadic>,
    pub output: syn::ReturnType,
}

// the ItemFn that supports default arguments
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct ItemFn {
    pub attrs: Vec<syn::Attribute>,
    pub vis: syn::Visibility,
    pub sig: Signature,
    pub block: Box<syn::Block>,
}

impl Parse for Assign {
    fn parse(input: ParseStream) -> Result<Self> {
        let eq_token: Token![=] = input.parse()?;
        let right: Box<syn::Expr> = input.parse()?;
        Ok(Self { eq_token, right })
    }
}

fn fn_arg_typed(input: ParseStream) -> Result<PatType> {
    // Hack to parse pre-2018 syntax in
    // test/ui/rfc-2565-param-attrs/param-attrs-pretty.rs
    // because the rest of the test case is valuable.
    if input.peek(syn::Ident) && input.peek2(Token![<]) {
        let span = input.fork().parse::<syn::Ident>()?.span();
        let mut pt = PatType {
            attrs: Vec::new(),
            pat: Box::new(syn::Pat::Wild(syn::PatWild {
                attrs: Vec::new(),
                underscore_token: Token![_](span),
            })),
            colon_token: Token![:](span),
            ty: input.parse()?,
            assign: None,
        };
        if input.peek(Token![=]) {
            pt.assign = Some(input.parse()?);
        }
        return Ok(pt);
    }

    let pat: Box<syn::Pat> = input.parse()?;
    let colon_token: syn::token::Colon = input.parse()?;
    let ty: syn::Type = match input.parse::<Option<Token![...]>>()? {
        Some(dot3) => syn::Type::Verbatim(variadic_to_tokens(&dot3)),
        None => input.parse()?,
    };
    println!("here {:?}", input);
    let assign: Option<Assign> = if input.peek(Token![=]) {
        Some(input.parse()?)
    } else {
        None
    };
    Ok(PatType {
        attrs: Vec::new(),
        pat,
        colon_token,
        ty: Box::new(ty),
        assign,
    })
}

impl Parse for FnArg {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(syn::Attribute::parse_outer)?;

        let ahead = input.fork();
        if let Ok(mut receiver) = ahead.parse::<syn::Receiver>() {
            if !ahead.peek(Token![:]) {
                input.advance_to(&ahead);
                receiver.attrs = attrs;
                return Ok(FnArg::Receiver(receiver));
            }
        }

        let mut typed = input.call(fn_arg_typed)?;
        typed.attrs = attrs;
        Ok(FnArg::Typed(typed))
    }
}

fn pop_variadic(args: &mut Punctuated<FnArg, Token![,]>) -> Option<syn::Variadic> {
    let trailing_punct = args.trailing_punct();

    let last = match args.last_mut()? {
        FnArg::Typed(last) => last,
        _ => return None,
    };

    let ty = match last.ty.as_ref() {
        syn::Type::Verbatim(ty) => ty,
        _ => return None,
    };

    let mut variadic = syn::Variadic {
        attrs: Vec::new(),
        dots: syn::parse2(ty.clone()).ok()?,
    };

    if let syn::Pat::Verbatim(pat) = last.pat.as_ref() {
        if pat.to_string() == "..." && !trailing_punct {
            variadic.attrs = std::mem::replace(&mut last.attrs, Vec::new());
            args.pop();
        }
    }

    Some(variadic)
}

fn parse_fn_args(input: ParseStream) -> Result<Punctuated<FnArg, Token![,]>> {
    let mut args = Punctuated::new();
    let mut has_receiver = false;

    while !input.is_empty() {
        let attrs = input.call(syn::Attribute::parse_outer)?;

        let arg = if let Some(dots) = input.parse::<Option<Token![...]>>()? {
            FnArg::Typed(PatType {
                attrs,
                pat: Box::new(syn::Pat::Verbatim(variadic_to_tokens(&dots))),
                colon_token: Token![:](dots.spans[0]),
                ty: Box::new(syn::Type::Verbatim(variadic_to_tokens(&dots))),
                assign: None,
            })
        } else {
            let mut arg: FnArg = input.parse()?;
            match &mut arg {
                FnArg::Receiver(receiver) if has_receiver => {
                    return Err(syn::Error::new(
                        receiver.self_token.span,
                        "unexpected second method receiver",
                    ));
                }
                FnArg::Receiver(receiver) if !args.is_empty() => {
                    return Err(syn::Error::new(
                        receiver.self_token.span,
                        "unexpected method receiver",
                    ));
                }
                FnArg::Receiver(receiver) => {
                    has_receiver = true;
                    receiver.attrs = attrs;
                }
                FnArg::Typed(arg) => arg.attrs = attrs,
            }
            arg
        };
        args.push_value(arg);

        if input.is_empty() {
            break;
        }

        let comma: Token![,] = input.parse()?;
        args.push_punct(comma);
    }

    Ok(args)
}

fn attrs(outer: Vec<syn::Attribute>, inner: Vec<syn::Attribute>) -> Vec<syn::Attribute> {
    let mut attrs = outer;
    attrs.extend(inner);
    attrs
}

impl Parse for ItemFn {
    fn parse(input: ParseStream) -> Result<Self> {
        println!("input {:?}", input);
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let vis: syn::Visibility = input.parse()?;
        let constness: Option<Token![const]> = input.parse()?;
        let asyncness: Option<Token![async]> = input.parse()?;
        let unsafety: Option<Token![unsafe]> = input.parse()?;
        let abi: Option<syn::Abi> = input.parse()?;
        let fn_token: Token![fn] = input.parse()?;
        let ident: syn::Ident = input.parse()?;
        let generics: syn::Generics = input.parse()?;

        let content;
        let paren_token = parenthesized!(content in input);
        println!("content {:?}", content);
        let mut inputs = parse_fn_args(&content)?;
        let variadic = pop_variadic(&mut inputs);

        let output: syn::ReturnType = input.parse()?;
        let where_clause: Option<syn::WhereClause> = input.parse()?;

        let content;
        let brace_token = braced!(content in input);
        let inner_attrs = content.call(syn::Attribute::parse_inner)?;
        let stmts = content.call(syn::Block::parse_within)?;

        Ok(ItemFn {
            attrs: attrs(outer_attrs, inner_attrs),
            vis,
            sig: Signature {
                constness,
                asyncness,
                unsafety,
                abi,
                fn_token,
                ident,
                paren_token,
                inputs,
                output,
                variadic,
                generics: syn::Generics {
                    where_clause,
                    ..generics
                },
            },
            block: Box::new(syn::Block { brace_token, stmts }),
        })
    }
}

impl ToTokens for Assign {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.eq_token.to_tokens(tokens);
        self.right.to_tokens(tokens);
    }
}

impl ToTokens for PatType {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.append_all(self.attrs.outer());
        self.pat.to_tokens(tokens);
        self.colon_token.to_tokens(tokens);
        self.ty.to_tokens(tokens);
        self.assign.to_tokens(tokens);
    }
}

impl ToTokens for FnArg {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match &*self {
            FnArg::Receiver(x) => x.to_tokens(tokens),
            FnArg::Typed(x) => x.to_tokens(tokens),
        }
    }
}

fn maybe_variadic_to_tokens(arg: &FnArg, tokens: &mut proc_macro2::TokenStream) -> bool {
    let arg = match arg {
        FnArg::Typed(arg) => arg,
        FnArg::Receiver(receiver) => {
            receiver.to_tokens(tokens);
            return false;
        }
    };

    match arg.ty.as_ref() {
        syn::Type::Verbatim(ty) if ty.to_string() == "..." => {
            match arg.pat.as_ref() {
                syn::Pat::Verbatim(pat) if pat.to_string() == "..." => {
                    tokens.append_all(arg.attrs.outer());
                    pat.to_tokens(tokens);
                }
                _ => arg.to_tokens(tokens),
            }
            true
        }
        _ => {
            arg.to_tokens(tokens);
            false
        }
    }
}

fn variadic_to_tokens(dots: &Token![...]) -> proc_macro2::TokenStream {
    proc_macro2::TokenStream::from_iter(vec![
        proc_macro2::TokenTree::Punct({
            let mut dot = proc_macro2::Punct::new('.', proc_macro2::Spacing::Joint);
            dot.set_span(dots.spans[0]);
            dot
        }),
        proc_macro2::TokenTree::Punct({
            let mut dot = proc_macro2::Punct::new('.', proc_macro2::Spacing::Joint);
            dot.set_span(dots.spans[1]);
            dot
        }),
        proc_macro2::TokenTree::Punct({
            let mut dot = proc_macro2::Punct::new('.', proc_macro2::Spacing::Joint);
            dot.set_span(dots.spans[2]);
            dot
        }),
    ])
}

impl ToTokens for Signature {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.constness.to_tokens(tokens);
        self.asyncness.to_tokens(tokens);
        self.unsafety.to_tokens(tokens);
        self.abi.to_tokens(tokens);
        self.fn_token.to_tokens(tokens);
        self.ident.to_tokens(tokens);
        self.generics.to_tokens(tokens);
        self.paren_token.surround(tokens, |tokens| {
            let mut last_is_variadic = false;
            for input in self.inputs.pairs() {
                match input {
                    Pair::Punctuated(input, comma) => {
                        maybe_variadic_to_tokens(input, tokens);
                        comma.to_tokens(tokens);
                    }
                    Pair::End(input) => {
                        last_is_variadic = maybe_variadic_to_tokens(input, tokens);
                    }
                }
            }
            if self.variadic.is_some() && !last_is_variadic {
                if !self.inputs.empty_or_trailing() {
                    <Token![,]>::default().to_tokens(tokens);
                }
                self.variadic.to_tokens(tokens);
            }
        });
        self.output.to_tokens(tokens);
        self.generics.where_clause.to_tokens(tokens);
    }
}

impl ToTokens for ItemFn {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.append_all(self.attrs.outer());
        self.vis.to_tokens(tokens);
        self.sig.to_tokens(tokens);
        self.block.brace_token.surround(tokens, |tokens| {
            tokens.append_all(self.attrs.inner());
            tokens.append_all(&self.block.stmts);
        });
    }
}
