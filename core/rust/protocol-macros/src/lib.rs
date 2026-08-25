use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::collections::HashSet;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    parse_macro_input, Error, Ident, ItemMod, ItemTrait, LitBool, LitInt, LitStr, Path, Result,
    Token, TraitItem,
};

struct NativeArgs {
    namespace: Option<LitStr>,
    name: Option<LitStr>,
    methods: Vec<LitStr>,
    whole_wasm_methods: Vec<NativeWholeWasmMethod>,
    provider: Option<Path>,
    availability: LitStr,
    capability: Option<LitStr>,
}

struct NativeWholeWasmMethod {
    name: LitStr,
    arity: LitInt,
}

impl Parse for NativeWholeWasmMethod {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let content;
        syn::parenthesized!(content in input);
        let name: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        let arity: LitInt = content.parse()?;
        Ok(Self { name, arity })
    }
}

impl Default for NativeArgs {
    fn default() -> Self {
        Self {
            namespace: None,
            name: None,
            methods: Vec::new(),
            whole_wasm_methods: Vec::new(),
            provider: None,
            availability: LitStr::new("portable", proc_macro2::Span::call_site()),
            capability: None,
        }
    }
}

impl Parse for NativeArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut args = Self::default();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "namespace" => args.namespace = Some(input.parse()?),
                "name" => args.name = Some(input.parse()?),
                "availability" => args.availability = input.parse()?,
                "capability" => args.capability = Some(input.parse()?),
                "methods" => {
                    let content;
                    syn::bracketed!(content in input);
                    args.methods = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect();
                }
                "whole_wasm_methods" => {
                    let content;
                    syn::bracketed!(content in input);
                    args.whole_wasm_methods =
                        Punctuated::<NativeWholeWasmMethod, Token![,]>::parse_terminated(&content)?
                            .into_iter()
                            .collect();
                }
                "provider" => args.provider = Some(input.parse()?),
                _ => return Err(Error::new(key.span(), "unknown hara_native option")),
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(args)
    }
}

fn native_availability_variant(value: &LitStr) -> Result<proc_macro2::TokenStream> {
    match value.value().as_str() {
        "portable" => Ok(quote!(crate::core::NativeAvailability::Portable)),
        "capability-gated" => Ok(quote!(crate::core::NativeAvailability::CapabilityGated)),
        "inventory-only" => Ok(quote!(crate::core::NativeAvailability::InventoryOnly)),
        _ => Err(Error::new_spanned(
            value,
            "availability must be portable, capability-gated, or inventory-only",
        )),
    }
}

#[proc_macro_attribute]
pub fn hara_native_registry(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemMod);
    match expand_native_registry(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_native_registry(mut input: ItemMod) -> Result<proc_macro2::TokenStream> {
    let (_, items) = input.content.as_mut().ok_or_else(|| {
        Error::new_spanned(
            &input.ident,
            "hara_native_registry requires an inline module",
        )
    })?;

    let mut declarations = Vec::new();
    let original_items = std::mem::take(items);
    let mut retained_items = Vec::with_capacity(original_items.len());
    let mut type_names = HashSet::new();
    for item in original_items {
        let syn::Item::Struct(native_type) = item else {
            retained_items.push(item);
            continue;
        };
        let Some(attribute_index) = native_type
            .attrs
            .iter()
            .position(|attribute| attribute.path().is_ident("hara_native"))
        else {
            retained_items.push(syn::Item::Struct(native_type));
            continue;
        };
        let attribute = &native_type.attrs[attribute_index];
        let args = attribute.parse_args::<NativeArgs>()?;
        let namespace = args.namespace.ok_or_else(|| {
            Error::new_spanned(&native_type.ident, "hara_native requires namespace")
        })?;
        let name = args
            .name
            .ok_or_else(|| Error::new_spanned(&native_type.ident, "hara_native requires name"))?;
        let provider = args.provider.ok_or_else(|| {
            Error::new_spanned(
                &native_type.ident,
                "hara_native requires a provider function",
            )
        })?;
        if namespace.value() != "std.native" {
            return Err(Error::new_spanned(
                &namespace,
                "hara_native declarations must use the std.native namespace",
            ));
        }
        if name.value().is_empty() {
            return Err(Error::new_spanned(
                &name,
                "hara_native requires a non-empty name",
            ));
        }
        if !type_names.insert(name.value()) {
            return Err(Error::new_spanned(&name, "duplicate hara_native type name"));
        }
        if args.methods.is_empty() {
            return Err(Error::new_spanned(
                &native_type.ident,
                "hara_native requires at least one method",
            ));
        }
        let mut method_names = HashSet::new();
        for method in &args.methods {
            if method.value().is_empty() {
                return Err(Error::new_spanned(
                    method,
                    "hara_native methods must not be empty",
                ));
            }
            if !method_names.insert(method.value()) {
                return Err(Error::new_spanned(
                    method,
                    "duplicate hara_native method name",
                ));
            }
        }
        for method in &args.whole_wasm_methods {
            if !method_names.contains(&method.name.value()) {
                return Err(Error::new_spanned(
                    &method.name,
                    "hara_native whole-Wasm methods must be listed in methods",
                ));
            }
            if method.arity.base10_parse::<u16>().is_err() {
                return Err(Error::new_spanned(
                    &method.arity,
                    "hara_native whole-Wasm method arity must fit u16",
                ));
            }
        }
        match (args.availability.value().as_str(), args.capability.as_ref()) {
            ("capability-gated", None) => {
                return Err(Error::new_spanned(
                    &args.availability,
                    "capability-gated hara_native declarations require a capability",
                ));
            }
            ("capability-gated", Some(capability)) if capability.value().is_empty() => {
                return Err(Error::new_spanned(
                    capability,
                    "capability-gated hara_native declarations require a non-empty capability",
                ));
            }
            ("portable" | "inventory-only", Some(capability)) if capability.value().is_empty() => {
                return Err(Error::new_spanned(
                    capability,
                    "portable and inventory-only hara_native declarations cannot declare an empty capability",
                ));
            }
            ("portable" | "inventory-only", Some(capability)) => {
                return Err(Error::new_spanned(
                    capability,
                    "only capability-gated hara_native declarations may declare a capability",
                ));
            }
            _ => {}
        }
        let availability = native_availability_variant(&args.availability)?;
        let capability = match args.capability {
            Some(value) => quote!(Some(#value)),
            None => quote!(None),
        };
        let methods = args.methods;
        let whole_wasm_methods = args
            .whole_wasm_methods
            .into_iter()
            .map(|method| {
                let name = method.name;
                let arity = method.arity.base10_parse::<u16>().expect("validated above");
                quote! {
                    crate::core::NativeOperationDeclaration {
                        name: #name,
                        arity: #arity,
                    }
                }
            })
            .collect::<Vec<_>>();
        declarations.push(quote! {
            crate::core::NativeDeclaration {
                namespace: #namespace,
                name: #name,
                methods: &[#(#methods),*],
                whole_wasm_methods: &[#(#whole_wasm_methods),*],
                provider: #provider,
                availability: #availability,
                capability: #capability,
            }
        });
    }
    *items = retained_items;

    if declarations.is_empty() {
        return Err(Error::new_spanned(
            &input.ident,
            "hara_native_registry requires at least one annotated struct",
        ));
    }

    let declaration_table =
        format_ident!("{}_DECLARATIONS", input.ident.to_string().to_uppercase());

    Ok(quote! {
        #input

        #[doc(hidden)]
        pub(crate) const #declaration_table: &[crate::core::NativeDeclaration] = &[
            #(#declarations),*
        ];
    })
}

struct ProtocolArgs {
    namespace: Option<LitStr>,
    name: Option<LitStr>,
    parents: Vec<LitStr>,
    inherited_methods: Vec<InheritedMethod>,
    availability: LitStr,
    capability: Option<LitStr>,
}

struct InheritedMethod {
    name: LitStr,
    rust_name: LitStr,
    arity: usize,
}

impl Parse for InheritedMethod {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let content;
        syn::parenthesized!(content in input);
        let name: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        let rust_name: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        let arity: LitInt = content.parse()?;
        Ok(Self {
            name,
            rust_name,
            arity: arity.base10_parse()?,
        })
    }
}

impl Default for ProtocolArgs {
    fn default() -> Self {
        Self {
            namespace: None,
            name: None,
            parents: Vec::new(),
            inherited_methods: Vec::new(),
            availability: LitStr::new("portable", proc_macro2::Span::call_site()),
            capability: None,
        }
    }
}

impl Parse for ProtocolArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut args = Self::default();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "namespace" => args.namespace = Some(input.parse()?),
                "name" => args.name = Some(input.parse()?),
                "availability" => args.availability = input.parse()?,
                "capability" => args.capability = Some(input.parse()?),
                "parents" => {
                    let content;
                    syn::bracketed!(content in input);
                    let values = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?;
                    args.parents = values.into_iter().collect();
                }
                "inherited_methods" => {
                    let content;
                    syn::bracketed!(content in input);
                    let values =
                        Punctuated::<InheritedMethod, Token![,]>::parse_terminated(&content)?;
                    args.inherited_methods = values.into_iter().collect();
                }
                _ => return Err(Error::new(key.span(), "unknown hara_protocol option")),
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(args)
    }
}

struct MethodArgs {
    value: Option<LitStr>,
    arity: Option<i64>,
    variadic: bool,
    min_arity: Option<usize>,
    max_arity: Option<usize>,
    whole_wasm: bool,
}

impl Default for MethodArgs {
    fn default() -> Self {
        Self {
            value: None,
            arity: None,
            variadic: false,
            min_arity: None,
            max_arity: None,
            whole_wasm: false,
        }
    }
}

impl Parse for MethodArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut args = Self::default();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "variadic" if !input.peek(Token![=]) => args.variadic = true,
                "whole_wasm" if !input.peek(Token![=]) => args.whole_wasm = true,
                _ => {
                    input.parse::<Token![=]>()?;
                    match key.to_string().as_str() {
                        "value" => args.value = Some(input.parse()?),
                        "arity" => {
                            let negative = if input.peek(Token![-]) {
                                input.parse::<Token![-]>()?;
                                true
                            } else {
                                false
                            };
                            let value: LitInt = input.parse()?;
                            let value = value.base10_parse::<i64>()?;
                            args.arity = Some(if negative { -value } else { value });
                        }
                        "variadic" => args.variadic = input.parse::<LitBool>()?.value,
                        "whole_wasm" => args.whole_wasm = input.parse::<LitBool>()?.value,
                        "min_arity" => {
                            let value: LitInt = input.parse()?;
                            args.min_arity = Some(value.base10_parse()?);
                        }
                        "max_arity" => {
                            let value: LitInt = input.parse()?;
                            args.max_arity = Some(value.base10_parse()?);
                        }
                        _ => return Err(Error::new(key.span(), "unknown hara_method option")),
                    }
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(args)
    }
}

fn availability_variant(value: &LitStr) -> Result<proc_macro2::TokenStream> {
    match value.value().as_str() {
        "portable" => Ok(quote!(
            crate::lang::protocol::ProtocolAvailability::Portable
        )),
        "capability-gated" => Ok(quote!(
            crate::lang::protocol::ProtocolAvailability::CapabilityGated
        )),
        "inventory-only" => Ok(quote!(
            crate::lang::protocol::ProtocolAvailability::InventoryOnly
        )),
        _ => Err(Error::new_spanned(
            value,
            "availability must be portable, capability-gated, or inventory-only",
        )),
    }
}

fn method_declaration(
    method: &syn::TraitItemFn,
    attr: &syn::Attribute,
) -> Result<(String, proc_macro2::TokenStream)> {
    let args = attr.parse_args::<MethodArgs>()?;
    let value = args
        .value
        .ok_or_else(|| Error::new_spanned(attr, "hara_method requires value"))?;
    let arity = args
        .arity
        .ok_or_else(|| Error::new_spanned(attr, "hara_method requires arity"))?;
    let rust_name = method.sig.ident.to_string();

    let arity_value = if args.variadic {
        if arity != -1 {
            return Err(Error::new_spanned(
                attr,
                "variadic hara methods must use arity = -1",
            ));
        }
        let minimum = args
            .min_arity
            .ok_or_else(|| Error::new_spanned(attr, "variadic hara methods require min_arity"))?;
        let maximum = args.max_arity;
        if maximum.is_some_and(|maximum| maximum < minimum) {
            return Err(Error::new_spanned(
                attr,
                "max_arity cannot be less than min_arity",
            ));
        }
        let maximum = match maximum {
            Some(value) => quote!(Some(#value)),
            None => quote!(None),
        };
        quote!(crate::lang::protocol::ProtocolArity::Variadic {
            minimum: #minimum,
            maximum: #maximum,
        })
    } else {
        if arity < 1 {
            return Err(Error::new_spanned(
                attr,
                "fixed hara method arity must include a receiver",
            ));
        }
        if args.min_arity.is_some() || args.max_arity.is_some() {
            return Err(Error::new_spanned(
                attr,
                "min_arity and max_arity are only valid for variadic methods",
            ));
        }
        let arity = arity as usize;
        quote!(crate::lang::protocol::ProtocolArity::Fixed(#arity))
    };

    let method_name = value.value();
    let whole_wasm = args.whole_wasm;
    Ok((
        method_name,
        quote! {
            crate::lang::protocol::ProtocolMethodDeclaration {
                name: #value,
                rust_name: #rust_name,
                arity: #arity_value,
                whole_wasm: #whole_wasm,
            }
        },
    ))
}

#[proc_macro_attribute]
pub fn hara_protocol(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ProtocolArgs);
    let mut input = parse_macro_input!(item as ItemTrait);
    match expand_protocol(args, &mut input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_protocol(args: ProtocolArgs, input: &mut ItemTrait) -> Result<proc_macro2::TokenStream> {
    let namespace = args
        .namespace
        .ok_or_else(|| Error::new_spanned(&input.ident, "hara_protocol requires namespace"))?;
    let name = args
        .name
        .ok_or_else(|| Error::new_spanned(&input.ident, "hara_protocol requires name"))?;
    let availability = availability_variant(&args.availability)?;
    let capability = match args.capability {
        Some(value) => quote!(Some(#value)),
        None => quote!(None),
    };
    let parents = args.parents;
    let inherited_methods = args.inherited_methods;
    let mut methods = Vec::new();
    let mut method_names = std::collections::HashSet::new();
    for item in &mut input.items {
        if let TraitItem::Fn(method) = item {
            let mut protocol_attribute = None;
            method.attrs.retain(|attribute| {
                if attribute.path().is_ident("hara_method") {
                    protocol_attribute = Some(attribute.clone());
                    false
                } else {
                    true
                }
            });
            if let Some(attribute) = protocol_attribute {
                let (method_name, declaration) = method_declaration(method, &attribute)?;
                if !method_names.insert(method_name) {
                    return Err(Error::new_spanned(
                        &method.sig.ident,
                        "duplicate Hara method in protocol",
                    ));
                }
                methods.push(declaration);
            }
        }
    }

    for inherited in inherited_methods {
        if !method_names.insert(inherited.name.value()) {
            return Err(Error::new_spanned(
                &input.ident,
                "duplicate Hara method in protocol",
            ));
        }
        let name = inherited.name;
        let rust_name = inherited.rust_name;
        let arity = inherited.arity;
        methods.push(quote! {
            crate::lang::protocol::ProtocolMethodDeclaration {
                name: #name,
                rust_name: #rust_name,
                arity: crate::lang::protocol::ProtocolArity::Fixed(#arity),
                whole_wasm: false,
            }
        });
    }

    Ok(quote! {
        #input

        inventory::submit! {
            crate::lang::protocol::ProtocolDeclaration {
                namespace: #namespace,
                name: #name,
                parents: &[#(#parents),*],
                availability: #availability,
                capability: #capability,
                methods: &[#(#methods),*],
            }
        }
    })
}

#[proc_macro_attribute]
pub fn hara_host_support(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
