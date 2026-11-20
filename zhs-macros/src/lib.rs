use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Error, Fields, FnArg, GenericArgument, ImplItem, ImplItemFn,
    ItemFn, ItemImpl, LitStr, Meta, Pat, PathArguments, Type, parse_macro_input, parse_quote,
};

#[proc_macro_attribute]
pub fn controller(attributes: TokenStream, item: TokenStream) -> TokenStream {
    let base = parse_macro_input!(attributes as LitStr).value();
    let mut item_impl = parse_macro_input!(item as ItemImpl);

    if let Err(error) = reject_generic_impl(&item_impl, "controller") {
        return error.to_compile_error().into();
    }

    match expand_controller(&base, &mut item_impl) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn injectable(_attributes: TokenStream, item: TokenStream) -> TokenStream {
    let mut item_impl = parse_macro_input!(item as ItemImpl);

    if let Err(error) = reject_generic_impl(&item_impl, "injectable") {
        return error.to_compile_error().into();
    }

    match expand_injectable_impl(&mut item_impl) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Attribute macros expand `impl` blocks without carrying generics over to
/// the generated trait impls, so reject generic self-types up front instead
/// of emitting broken code.
fn reject_generic_impl(item_impl: &ItemImpl, macro_name: &str) -> Result<(), Error> {
    if !item_impl.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &item_impl.generics,
            format!("{macro_name} does not support generic impls"),
        ));
    }

    Ok(())
}

fn expand_controller(
    base: &str,
    item_impl: &mut ItemImpl,
) -> Result<proc_macro2::TokenStream, Error> {
    let self_ty = item_impl.self_ty.clone();
    let type_ident = type_ident(&self_ty)?;

    let mut routes = Vec::new();
    let mut constructor = None;

    for item in &mut item_impl.items {
        let ImplItem::Fn(method) = item else {
            continue;
        };

        let is_inject = has_attr(&method.attrs, "inject");
        let actions = collect_actions(&method.attrs)?;

        if is_inject {
            constructor = Some(method.clone());
        }

        for (http_method, path) in actions {
            routes.push(expand_route(
                &self_ty,
                &type_ident,
                base,
                method,
                &http_method,
                &path,
            )?);
        }

        method.attrs.retain(|attribute| {
            !is_http_attr(attribute)
                && !is_auth_attr(attribute)
                && !has_attr_name(attribute, "inject")
        });

        strip_from_attributes(method);
    }

    let controller_impl = quote! {
        impl zhs_framework::http::Controller for #self_ty {
            fn register(router: &mut zhs_framework::http::Router) {
                #(#routes)*
            }
        }
    };

    let injectable_impl = expand_injectable(&self_ty, constructor.as_ref(), false)?;

    Ok(quote! {
        #item_impl
        #controller_impl
        #injectable_impl
    })
}

fn expand_injectable_impl(item_impl: &mut ItemImpl) -> Result<proc_macro2::TokenStream, Error> {
    let self_ty = item_impl.self_ty.clone();
    type_ident(&self_ty)?;

    let mut constructor = None;

    for item in &mut item_impl.items {
        if let ImplItem::Fn(method) = item
            && has_attr(&method.attrs, "inject")
        {
            constructor = Some(method.clone());
            method
                .attrs
                .retain(|attribute| !has_attr_name(attribute, "inject"));
        }
    }

    let injectable_impl = expand_injectable(&self_ty, constructor.as_ref(), true)?;

    Ok(quote! {
        #item_impl
        #injectable_impl
    })
}

fn expand_injectable(
    self_ty: &Type,
    constructor: Option<&ImplItemFn>,
    require_constructor: bool,
) -> Result<proc_macro2::TokenStream, Error> {
    let Some(constructor) = constructor else {
        if require_constructor {
            return Err(Error::new_spanned(
                self_ty,
                "`#[injectable]` requires a method marked with `#[inject]`",
            ));
        }

        return Ok(quote! {
            impl zhs_framework::di::Injectable for #self_ty {
                fn create(
                    _services: &dyn zhs_framework::http::ServiceResolver,
                ) -> Result<Self, zhs_framework::di::DiError> {
                    Ok(Self::default())
                }
            }
        });
    };

    let constructor_name = &constructor.sig.ident;
    let mut bindings = Vec::new();
    let mut arguments = Vec::new();

    for input in &constructor.sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            return Err(Error::new_spanned(
                input,
                "`#[inject]` constructor must not take `self`",
            ));
        };

        let name = pat_ident(&pat_type.pat)?;

        if let Some(inner) = arc_inner(&pat_type.ty) {
            bindings.push(quote! {
                let #name = zhs_framework::http::resolve_required::<#inner>(__zhs_services)
                    .map_err(|error| zhs_framework::di::DiError::MissingService(error.to_string()))?;
            });
        } else if let Some(inner) = type_argument(&pat_type.ty, "Logger") {
            let ty = &pat_type.ty;
            bindings.push(quote! {
                let __zhs_logger_factory =
                    zhs_framework::http::resolve_required::<zhs_framework::logging::LoggerFactory>(__zhs_services)
                        .map_err(|error| zhs_framework::di::DiError::MissingService(error.to_string()))?;
                let #name: #ty = zhs_framework::logging::Logger::<#inner>::from_factory(
                    &__zhs_logger_factory,
                );
            });
        } else {
            return Err(Error::new_spanned(
                &pat_type.ty,
                "constructor dependencies must be `Arc<T>` or `Logger<T>`",
            ));
        }

        arguments.push(quote! { #name });
    }

    Ok(quote! {
        impl zhs_framework::di::Injectable for #self_ty {
            fn create(
                __zhs_services: &dyn zhs_framework::http::ServiceResolver,
            ) -> Result<Self, zhs_framework::di::DiError> {
                #(#bindings)*
                Ok(Self::#constructor_name(#(#arguments),*))
            }
        }
    })
}

fn expand_route(
    self_ty: &Type,
    type_ident: &syn::Ident,
    base: &str,
    method: &ImplItemFn,
    http_method: &str,
    route_path: &str,
) -> Result<proc_macro2::TokenStream, Error> {
    let method_name = &method.sig.ident;
    let auth_check = authorization_check(&method.attrs)?;
    let mut bindings = Vec::new();
    let mut arguments = Vec::new();

    for input in &method.sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            continue;
        };

        let name = pat_ident(&pat_type.pat)?;
        let ty = &pat_type.ty;
        let key = name.to_string();
        let key_literal = LitStr::new(&key, name.span());

        if has_attr(&pat_type.attrs, "from_route") {
            bindings.push(quote! {
                let #name: #ty = __zhs_ctx.route_value::<#ty>(#key_literal)?;
            });
        } else if has_attr(&pat_type.attrs, "from_files") {
            bindings.push(quote! {
                let #name: #ty = __zhs_ctx.multipart_files()?;
            });
        } else if has_attr(&pat_type.attrs, "from_query") {
            if let Some(inner) = option_inner(ty) {
                bindings.push(quote! {
                    let #name: #ty = __zhs_ctx.query_value::<#inner>(#key_literal)?;
                });
            } else {
                bindings.push(quote! {
                    let #name: #ty = __zhs_ctx.query_value::<#ty>(#key_literal)?
                        .ok_or_else(|| zhs_framework::http::HttpError::BadRequest(
                            ::std::format!("missing query parameter `{}`", #key_literal)
                        ))?;
                });
            }
        } else if has_attr(&pat_type.attrs, "from_body") {
            if is_type(ty, "String") {
                bindings.push(quote! {
                    let #name: String = __zhs_ctx.body_text()?;
                });
            } else if is_vec_u8(ty) {
                bindings.push(quote! {
                    let #name: Vec<u8> = __zhs_ctx.request.body.clone();
                });
            } else {
                bindings.push(quote! {
                    let #name: #ty = zhs_framework::json::from_str::<#ty>(&__zhs_ctx.body_text()?)
                        .map_err(|error| zhs_framework::http::HttpError::BadRequest(error.to_string()))?;
                });
            }
        } else if has_attr(&pat_type.attrs, "from_user") {
            bindings.push(quote! {
                let #name: #ty = __zhs_ctx.user.clone();
            });
        } else if has_attr(&pat_type.attrs, "from_services") {
            if let Some(inner) = arc_inner(ty) {
                bindings.push(quote! {
                    let #name: #ty = __zhs_ctx.resolve::<#inner>()?;
                });
            } else if let Some(inner) = type_argument(ty, "Logger") {
                bindings.push(quote! {
                    let __zhs_logger_factory = __zhs_ctx.resolve::<zhs_framework::logging::LoggerFactory>()?;
                    let #name: #ty = zhs_framework::logging::Logger::<#inner>::from_factory(
                        &__zhs_logger_factory,
                    );
                });
            } else {
                return Err(Error::new_spanned(
                    ty,
                    "`#[from_services]` requires `Arc<T>` or `Logger<T>`",
                ));
            }
        } else {
            return Err(Error::new_spanned(
                ty,
                "controller parameters need `#[from_route]`, `#[from_query]`, `#[from_body]` or `#[from_services]`",
            ));
        }

        arguments.push(quote! { #name });
    }

    let path = combine_path(base, route_path, type_ident, method_name);
    let path_literal = LitStr::new(&path, Span::call_site());
    let http_method = syn::Ident::new(http_method, Span::call_site());
    let operation_id = LitStr::new(&method_name.to_string(), Span::call_site());
    let summary = LitStr::new(&doc_summary(&method.attrs), Span::call_site());
    let type_name = type_ident.to_string();
    let tag = type_name
        .strip_suffix("Controller")
        .unwrap_or(&type_name)
        .to_owned();
    let tag = LitStr::new(&tag, Span::call_site());
    let secured = has_attr(&method.attrs, "authorize");

    Ok(quote! {
        {
            let handler: zhs_framework::http::HandlerFn = ::std::sync::Arc::new(|__zhs_ctx| {
                ::std::boxed::Box::pin(async move {
                    #auth_check
                    let __zhs_controller = __zhs_ctx.resolve::<#self_ty>()?;
                    #(#bindings)*
                    let __zhs_result = __zhs_controller.#method_name(#(#arguments),*).await;
                    Ok(zhs_framework::into_response(__zhs_result))
                })
            });

            router.route(zhs_framework::http::Method::#http_method, #path_literal, handler);
            router.document(zhs_framework::http::RouteDoc::new(
                zhs_framework::http::Method::#http_method,
                #path_literal,
                #operation_id,
                #summary,
                ::std::vec![#tag.to_owned()],
                #secured,
            ));
        }
    })
}

fn doc_summary(attributes: &[Attribute]) -> String {
    for attribute in attributes {
        if !attribute.path().is_ident("doc") {
            continue;
        }

        let Meta::NameValue(name_value) = &attribute.meta else {
            continue;
        };
        let syn::Expr::Lit(expression) = &name_value.value else {
            continue;
        };
        let syn::Lit::Str(text) = &expression.lit else {
            continue;
        };

        return text
            .value()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
    }

    String::new()
}

fn authorization_check(attributes: &[Attribute]) -> Result<proc_macro2::TokenStream, Error> {
    for attribute in attributes {
        if !attribute.path().is_ident("authorize") {
            continue;
        }

        let mut policy: Option<String> = None;
        let mut roles: Vec<String> = Vec::new();

        if let Meta::List(_) = &attribute.meta {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("policy") {
                    let value: LitStr = meta.value()?.parse()?;
                    policy = Some(value.value());
                } else if meta.path.is_ident("roles") {
                    let value: LitStr = meta.value()?.parse()?;
                    roles.extend(
                        value
                            .value()
                            .split(',')
                            .map(str::trim)
                            .filter(|role| !role.is_empty())
                            .map(str::to_owned),
                    );
                } else {
                    return Err(meta.error("unsupported authorize attribute"));
                }

                Ok(())
            })?;
        }

        let policy = match policy {
            Some(policy) => {
                let policy = LitStr::new(&policy, Span::call_site());
                quote! { Some(#policy) }
            }
            None => quote! { None },
        };
        let roles = roles
            .iter()
            .map(|role| LitStr::new(role, Span::call_site()))
            .collect::<Vec<_>>();

        return Ok(quote! {
            if let Err(error) = zhs_framework::auth::authorize(&__zhs_ctx, #policy, &[#(#roles),*]) {
                return Ok(zhs_framework::http::IntoResponse::into_response(error));
            }
        });
    }

    Ok(quote! {})
}

fn combine_path(
    base: &str,
    route_path: &str,
    type_ident: &syn::Ident,
    method_ident: &syn::Ident,
) -> String {
    let controller = type_ident.to_string();
    let controller = controller
        .strip_suffix("Controller")
        .unwrap_or(&controller)
        .to_ascii_lowercase();
    let action = method_ident.to_string().to_ascii_lowercase();

    let replace = |value: &str| {
        value
            .replace("[controller]", &controller)
            .replace("[action]", &action)
    };

    let base = replace(base).trim_end_matches('/').to_owned();
    let route_path = replace(route_path).trim_start_matches('/').to_owned();

    if route_path.is_empty() {
        if base.is_empty() {
            "/".to_owned()
        } else {
            base
        }
    } else if base.is_empty() {
        format!("/{route_path}")
    } else {
        format!("{base}/{route_path}")
    }
}

fn collect_actions(attributes: &[Attribute]) -> Result<Vec<(String, String)>, Error> {
    let mut actions = Vec::new();

    for attribute in attributes {
        let Some(ident) = attribute.path().get_ident() else {
            continue;
        };

        let ident = ident.to_string();
        let method = match ident.as_str() {
            "http_get" => "Get",
            "http_post" => "Post",
            "http_put" => "Put",
            "http_patch" => "Patch",
            "http_delete" => "Delete",
            "http_head" => "Head",
            "http_options" => "Options",
            other if other.starts_with("http_") => {
                return Err(Error::new_spanned(
                    attribute,
                    format!("unsupported HTTP method attribute `{other}`"),
                ));
            }
            _ => continue,
        };

        let path = match &attribute.meta {
            Meta::Path(_) => String::new(),
            Meta::List(_) => {
                let path: LitStr = attribute.parse_args()?;
                path.value()
            }
            Meta::NameValue(_) => {
                return Err(Error::new_spanned(
                    attribute,
                    "expected `#[http_get]` or `#[http_get(\"/path\")]`",
                ));
            }
        };

        actions.push((method.to_owned(), path));
    }

    Ok(actions)
}

fn strip_from_attributes(method: &mut ImplItemFn) {
    for input in &mut method.sig.inputs {
        if let FnArg::Typed(pat_type) = input {
            pat_type.attrs.retain(|attribute| !is_from_attr(attribute));
        }
    }
}

fn type_ident(self_ty: &Type) -> Result<syn::Ident, Error> {
    let Type::Path(path) = self_ty else {
        return Err(Error::new_spanned(
            self_ty,
            "controllers must be declared on a named type",
        ));
    };

    path.path
        .segments
        .last()
        .map(|segment| segment.ident.clone())
        .ok_or_else(|| Error::new_spanned(self_ty, "missing controller type name"))
}

fn pat_ident(pat: &Pat) -> Result<&syn::Ident, Error> {
    match pat {
        Pat::Ident(pat) => Ok(&pat.ident),
        _ => Err(Error::new_spanned(
            pat,
            "controller parameters must be simple identifiers",
        )),
    }
}

fn arc_inner(ty: &Type) -> Option<&Type> {
    type_argument(ty, "Arc")
}

fn option_inner(ty: &Type) -> Option<&Type> {
    type_argument(ty, "Option")
}

fn type_argument<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;

    if segment.ident != name {
        return None;
    }

    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };

    match arguments.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

fn is_type(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(path) if path.path.is_ident(name))
}

fn is_vec_u8(ty: &Type) -> bool {
    type_argument(ty, "Vec").is_some_and(|inner| is_type(inner, "u8"))
}

fn has_attr(attributes: &[Attribute], name: &str) -> bool {
    attributes
        .iter()
        .any(|attribute| has_attr_name(attribute, name))
}

fn has_attr_name(attribute: &Attribute, name: &str) -> bool {
    attribute.path().is_ident(name)
}

fn is_http_attr(attribute: &Attribute) -> bool {
    attribute
        .path()
        .get_ident()
        .is_some_and(|ident| ident.to_string().starts_with("http_"))
}

fn is_auth_attr(attribute: &Attribute) -> bool {
    attribute.path().is_ident("authorize") || attribute.path().is_ident("allow_anonymous")
}

fn is_from_attr(attribute: &Attribute) -> bool {
    attribute
        .path()
        .get_ident()
        .is_some_and(|ident| ident.to_string().starts_with("from_"))
}

#[proc_macro_derive(ToJson, attributes(json))]
pub fn derive_to_json(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand_to_json(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(FromJson, attributes(json))]
pub fn derive_from_json(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand_from_json(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[derive(Default)]
struct JsonFieldOptions {
    rename: Option<String>,
    skip: bool,
    default: bool,
}

fn expand_to_json(input: &DeriveInput) -> Result<proc_macro2::TokenStream, Error> {
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "wire 1 derives do not support generic structs",
        ));
    }

    let name = &input.ident;
    let fields = named_fields(input)?;
    let mut inserts = Vec::new();

    for field in fields {
        let options = json_field_options(&field.attrs)?;
        if options.skip {
            continue;
        }

        let field_name = field
            .ident
            .as_ref()
            .ok_or_else(|| Error::new_spanned(field, "expected a named field"))?;
        let key = options.rename.unwrap_or_else(|| field_name.to_string());

        inserts.push(quote! {
            object.insert(#key, zhs_framework::json::ToJson::to_json(&self.#field_name));
        });
    }

    Ok(quote! {
        impl zhs_framework::json::ToJson for #name {
            fn to_json(&self) -> zhs_framework::json::Value {
                let mut object = zhs_framework::json::Object::new();
                #(#inserts)*
                zhs_framework::json::Value::Object(object)
            }
        }
    })
}

fn expand_from_json(input: &DeriveInput) -> Result<proc_macro2::TokenStream, Error> {
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "wire 1 derives do not support generic structs",
        ));
    }

    let name = &input.ident;
    let fields = named_fields(input)?;
    let mut initializers = Vec::new();

    for field in fields {
        let options = json_field_options(&field.attrs)?;
        let field_name = field
            .ident
            .as_ref()
            .ok_or_else(|| Error::new_spanned(field, "expected a named field"))?;
        let key = options.rename.unwrap_or_else(|| field_name.to_string());

        if options.skip {
            initializers.push(quote! {
                #field_name: ::std::default::Default::default()
            });
            continue;
        }

        if option_inner(&field.ty).is_some() {
            initializers.push(quote! {
                #field_name: match object.get(#key)
                    .or_else(|| object.get_ignore_ascii_case(#key))
                {
                    Some(value) if !value.is_null() => Some(
                        zhs_framework::json::FromJson::from_json(value)
                            .map_err(|error| error.prepend_field(#key))?
                    ),
                    _ => None,
                }
            });
        } else if options.default {
            initializers.push(quote! {
                #field_name: match object.get(#key)
                    .or_else(|| object.get_ignore_ascii_case(#key))
                {
                    Some(value) if !value.is_null() => zhs_framework::json::FromJson::from_json(value)
                        .map_err(|error| error.prepend_field(#key))?,
                    _ => ::std::default::Default::default(),
                }
            });
        } else {
            initializers.push(quote! {
                #field_name: {
                    let value = object.get(#key)
                        .or_else(|| object.get_ignore_ascii_case(#key))
                        .ok_or_else(|| {
                            zhs_framework::json::JsonDecodeError::missing(#key)
                        })?;
                    zhs_framework::json::FromJson::from_json(value)
                        .map_err(|error| error.prepend_field(#key))?
                }
            });
        }
    }

    Ok(quote! {
        impl zhs_framework::json::FromJson for #name {
            fn from_json(value: &zhs_framework::json::Value) -> Result<Self, zhs_framework::json::JsonDecodeError> {
                let object = value.as_object().ok_or_else(|| {
                    zhs_framework::json::JsonDecodeError::expected("object", value)
                })?;

                Ok(Self {
                    #(#initializers),*
                })
            }
        }
    })
}

fn named_fields(
    input: &DeriveInput,
) -> Result<&syn::punctuated::Punctuated<syn::Field, syn::token::Comma>, Error> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            input,
            "wire 1 derives only support named structs",
        ));
    };

    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new_spanned(
            &data.fields,
            "wire 1 derives only support named structs",
        ));
    };

    Ok(&fields.named)
}

fn json_field_options(attributes: &[Attribute]) -> Result<JsonFieldOptions, Error> {
    let mut options = JsonFieldOptions::default();

    for attribute in attributes {
        if !attribute.path().is_ident("json") {
            continue;
        }

        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value: LitStr = meta.value()?.parse()?;
                options.rename = Some(value.value());
            } else if meta.path.is_ident("skip") {
                options.skip = true;
            } else if meta.path.is_ident("default") {
                options.default = true;
            } else {
                return Err(meta.error("unsupported json attribute"));
            }

            Ok(())
        })?;
    }

    Ok(options)
}

#[proc_macro_derive(FromRow, attributes(table, column))]
pub fn derive_from_row(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand_from_row(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_from_row(input: &DeriveInput) -> Result<proc_macro2::TokenStream, Error> {
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "FromRow does not support generic structs",
        ));
    }

    let name = &input.ident;
    let fields = named_fields(input)?;
    let mut initializers = Vec::new();

    for field in fields {
        let field_name = field
            .ident
            .as_ref()
            .ok_or_else(|| Error::new_spanned(field, "expected a named field"))?;
        let column = column_name(&field.attrs)?.unwrap_or_else(|| field_name.to_string());
        let ty = &field.ty;

        initializers.push(quote! {
            #field_name: row.get::<#ty>(#column)?
        });
    }

    let table = table_name(&input.attrs)?;
    let table_impl = table.map(|table| {
        quote! {
            impl #name {
                pub const TABLE: &'static str = #table;
            }
        }
    });

    Ok(quote! {
        impl zhs_framework::entity::FromRow for #name {
            fn from_row(row: &zhs_framework::entity::Row) -> Result<Self, zhs_framework::entity::Error> {
                Ok(Self {
                    #(#initializers),*
                })
            }
        }

        #table_impl
    })
}

fn column_name(attributes: &[Attribute]) -> Result<Option<String>, Error> {
    for attribute in attributes {
        if attribute.path().is_ident("column") {
            let value: LitStr = attribute.parse_args()?;
            return Ok(Some(value.value()));
        }
    }

    Ok(None)
}

fn table_name(attributes: &[Attribute]) -> Result<Option<String>, Error> {
    for attribute in attributes {
        if attribute.path().is_ident("table") {
            let value: LitStr = attribute.parse_args()?;
            return Ok(Some(value.value()));
        }
    }

    Ok(None)
}

#[proc_macro_attribute]
pub fn test(_attributes: TokenStream, item: TokenStream) -> TokenStream {
    let mut function = parse_macro_input!(item as ItemFn);

    if function.sig.asyncness.is_some() {
        let body = function.block;
        function.sig.asyncness = None;
        function.block = Box::new(parse_quote!({
            zhs_framework::testing::block_on(async move #body)
        }));
    }

    function.attrs.push(parse_quote!(#[test]));

    quote! { #function }.into()
}
