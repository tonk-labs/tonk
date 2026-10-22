// Copyright 2026 The Dialog DB Project
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A subset of `wasi:http/types@0.3.0`, defined on a polyfill
//! [`Linker`], that serves both contexts: the page, whose elements send
//! requests through `wasi:http/client`, and the service worker, whose
//! routes answer them through `wasi:http/handler`.
//!
//! The subset is what the demo's authoring library and Zena's standard
//! library call: fields, requests, and responses, and their bodies as
//! streams. Every other function of the interface is in the linker, so a
//! component that imports the whole interface links, and it returns an
//! error when it is called. So a change in Zena's output shows as a
//! failure, not a silent wrong answer.
//!
//! The host keeps what each resource holds in an [`HttpTable`] in the
//! store's data, by the resource's representation. A body that crosses
//! the host is whole bytes: the host reads a guest's body stream to its
//! end, and writes a body to a guest as one stream that ends.
//!
//! The function types come from the component that imports the
//! interface, so the definitions here never restate the WIT; the host
//! checks each value it is given against the shape it reads. A
//! component's own resource types stand in those function types, so
//! the host replaces each with the identity it registers for it, with
//! [`host_signature`], before it defines a function.

use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use wcmp::{
    Component, ComponentType, Error, ExternType, FixedLengthListType, FunctionParameter,
    FunctionType, FutureAny, FutureConsumer, FutureReader, FutureType, HostCall,
    InterfaceIdentifier, Lift, Linker, ListType, Lower, MapType, OptionType, RecordField,
    RecordType, ResourceAny, ResourceDynamic, ResourceType, ResultType, Source, StoreContext,
    StreamAny, StreamConsumer, StreamReader, StreamResult, StreamType, TupleType, Val, ValueType,
    VariantCase, VariantType,
};

mod body;
mod fields;
mod http_host;
mod http_table;
mod request;
mod response;
mod types;

pub use body::Body;
pub use fields::Fields;
pub use http_host::HttpHost;
pub use http_table::HttpTable;
pub use request::Request;
pub use response::Response;
pub use types::Types;

/// The interface this module defines.
pub const TYPES: &str = "wasi:http/types@0.3.0";

/// A handle the host makes for the response of `rep`, which it answers a
/// guest's `wasi:http/client.send` with.
///
/// # Errors
///
/// The polyfill's error when it cannot make the handle.
pub fn response_handle<T: HttpHost>(
    store: &mut StoreContext<'_, T>,
    rep: u32,
) -> Result<ResourceAny, Error> {
    let types = store.data_mut().http().types()?;
    ResourceDynamic::new_own(rep, types.response).try_into_resource_any(store)
}

/// Define [`TYPES`] in `linker`, with the types `component` imports it
/// under, and answer the resource types for the store's
/// [`HttpTable`]. A component that does not import the interface leaves
/// the linker as it was, and has no resource types.
///
/// # Errors
///
/// The polyfill's error when it refuses a definition.
pub fn define<T: HttpHost>(
    linker: &mut Linker<T>,
    component: &Component,
) -> Result<Option<Types>, Error> {
    let Some(items) = component.imports.iter().find_map(|import| {
        match (&import.ty, import.name.to_string() == TYPES) {
            (ExternType::Instance(instance), true) => Some(instance.items.clone()),
            _ => None,
        }
    }) else {
        return Ok(None);
    };
    let identifier: InterfaceIdentifier = TYPES.parse().expect("the interface's name parses");
    let identities = identities(component);
    let mut types = linker.instance(&identifier);
    for name in RESOURCES {
        types.resource(name, identity(name), destructor::<T>(name))?;
    }
    let registered = Types {
        fields: dynamic("fields"),
        request: dynamic("request"),
        response: dynamic("response"),
    };
    for item in &items {
        let ExternType::Function(ty) = &item.ty else {
            continue;
        };
        let name = item.name.clone();
        let ty: FunctionType = host_function_type(&identities, ty);
        let function = name.clone();
        types.func_new(name, ty, move |mut call, args, results| {
            let answer = call_types(&mut call, &function, args)?;
            if let (Some(answer), Some(slot)) = (answer, results.first_mut()) {
                *slot = answer;
            }
            Ok(())
        })?;
    }
    // Another interface of `wasi:http`, such as `client` or `handler`,
    // names the resources it uses from `types`. Each such name is the
    // same resource type, so it is registered with the same identity.
    for import in &component.imports {
        let ExternType::Instance(instance) = &import.ty else {
            continue;
        };
        let name = import.name.to_string();
        if name == TYPES {
            continue;
        }
        let Ok(identifier) = name.parse::<InterfaceIdentifier>() else {
            continue;
        };
        for item in &instance.items {
            let (ExternType::Resource(_) | ExternType::ResourceEquals(_)) = &item.ty else {
                continue;
            };
            if let Some(resource) = RESOURCES.into_iter().find(|name| *name == item.name) {
                linker.instance(&identifier).resource(
                    item.name.clone(),
                    identity(resource),
                    destructor::<T>(resource),
                )?;
            }
        }
    }
    Ok(Some(registered))
}

/// The resources of [`TYPES`]. The host makes the identity of each at
/// run time, keyed on its index in this list.
const RESOURCES: [&str; 4] = ["fields", "request", "response", "request-options"];

/// The identity the host registers the resource `name` of [`TYPES`]
/// with.
///
/// # Panics
///
/// When `name` is not one of [`RESOURCES`].
fn identity(name: &str) -> ResourceType {
    ResourceType::host_dynamic(dynamic(name))
}

/// The dynamic resource type of the resource `name` of [`TYPES`]: its
/// index in [`RESOURCES`].
///
/// # Panics
///
/// When `name` is not one of [`RESOURCES`].
fn dynamic(name: &str) -> u32 {
    let index = RESOURCES
        .iter()
        .position(|resource| *resource == name)
        .expect("the resource is one of `wasi:http/types`");
    u32::try_from(index).expect("the list is short")
}

/// The destructor of the resource `name`, which forgets what the host
/// keeps for the representation it drops.
fn destructor<T: HttpHost>(
    name: &'static str,
) -> impl for<'c> Fn(StoreContext<'c, T>, u32) -> Result<(), Error> + Send + Sync + 'static {
    move |mut store, rep| {
        let table = store.data_mut().http();
        match name {
            "fields" => drop(table.fields.remove(&rep)),
            "request" => drop(table.requests.remove(&rep)),
            "response" => drop(table.responses.remove(&rep)),
            _ => {}
        }
        Ok(())
    }
}

/// The identity the host registers for each resource type `component`
/// declares in an interface it imports, by the resource type's
/// identity in the component.
fn identities(component: &Component) -> HashMap<ResourceType, ResourceType> {
    let mut identities = HashMap::new();
    for import in &component.imports {
        let ExternType::Instance(instance) = &import.ty else {
            continue;
        };
        for item in &instance.items {
            let (ExternType::Resource(declared) | ExternType::ResourceEquals(declared)) = &item.ty
            else {
                continue;
            };
            if RESOURCES.contains(&item.name.as_str()) {
                identities.insert(*declared, identity(&item.name));
            }
        }
    }
    identities
}

/// The type the host defines a function of `component` with, whose
/// type `ty` is: the same type, with each resource type the component
/// declares replaced by the identity the host registers for it.
pub fn host_signature(component: &Component, ty: &FunctionType) -> FunctionType {
    host_function_type(&identities(component), ty)
}

/// [`host_signature`], with the identities already read.
pub(crate) fn host_function_type(
    identities: &HashMap<ResourceType, ResourceType>,
    ty: &FunctionType,
) -> FunctionType {
    FunctionType {
        parameters: ty
            .parameters
            .iter()
            .map(|parameter| FunctionParameter {
                name: parameter.name.clone(),
                ty: host_value_type(identities, &parameter.ty),
            })
            .collect(),
        result: ty
            .result
            .as_ref()
            .map(|result| host_value_type(identities, result)),
        async_: ty.async_,
    }
}

/// [`host_function_type`] for one value type.
fn host_value_type(identities: &HashMap<ResourceType, ResourceType>, ty: &ValueType) -> ValueType {
    let inner = |ty: &ValueType| host_value_type(identities, ty);
    let resource = |declared: &ResourceType| identities.get(declared).copied().unwrap_or(*declared);
    match ty {
        ValueType::Own(declared) => ValueType::Own(resource(declared)),
        ValueType::Borrow(declared) => ValueType::Borrow(resource(declared)),
        ValueType::Record(record) => ValueType::Record(RecordType::new(
            record
                .fields()
                .iter()
                .map(|field| RecordField::new(field.name(), inner(field.ty()))),
        )),
        ValueType::Variant(variant) => ValueType::Variant(VariantType::new(
            variant
                .cases()
                .iter()
                .map(|case| VariantCase::new(case.name(), case.payload().map(inner))),
        )),
        ValueType::List(list) => ValueType::List(ListType::new(inner(list.element()))),
        ValueType::Map(map) => ValueType::Map(MapType::new(inner(map.key()), inner(map.value()))),
        ValueType::FixedLengthList(list) => ValueType::FixedLengthList(FixedLengthListType::new(
            inner(list.element()),
            list.length(),
        )),
        ValueType::Option(option) => ValueType::Option(OptionType::new(inner(option.payload()))),
        ValueType::Result(result) => ValueType::Result(ResultType::new(
            result.ok().map(inner),
            result.err().map(inner),
        )),
        ValueType::Tuple(tuple) => {
            ValueType::Tuple(TupleType::new(tuple.elements().iter().map(inner)))
        }
        ValueType::Stream(stream) => {
            ValueType::Stream(StreamType::new(stream.payload().map(inner)))
        }
        ValueType::Future(future) => {
            ValueType::Future(FutureType::new(future.payload().map(inner)))
        }
        other => other.clone(),
    }
}

/// What a function of [`TYPES`] that the subset leaves out answers.
fn unsupported(function: &str) -> Error {
    Error::Unsupported {
        feature: format!("`{TYPES}#{function}` in the demo's subset of `wasi:http`"),
    }
}

/// The error for an argument of the wrong shape.
fn malformed(function: &str, args: &[Val]) -> Error {
    Error::Internal {
        message: format!("`{TYPES}#{function}` was given {args:?}"),
    }
}

/// The representation of the handle argument `val`, if it is one.
///
/// An owned handle leaves the store's host table, because the host
/// keeps what the resource holds by its representation and takes it
/// over: a function that takes an `own` consumes the resource. A
/// borrow stays where it is.
///
/// # Errors
///
/// The polyfill's error when the handle is not one of a dynamic host
/// resource type, or the host table no longer holds it.
fn rep_of<T: 'static>(
    store: &mut StoreContext<'_, T>,
    val: Option<&Val>,
) -> Result<Option<u32>, Error> {
    match val {
        Some(Val::Resource(handle)) => Ok(Some(handle.try_into_resource_dynamic(store)?.rep())),
        _ => Ok(None),
    }
}

/// A new owned handle of the resource `rep`, of the dynamic resource
/// type `ty`, as the value a host function answers.
fn new_handle<T: 'static>(
    store: &mut StoreContext<'_, T>,
    rep: u32,
    ty: u32,
) -> Result<Val, Error> {
    Ok(Val::Resource(
        ResourceDynamic::new_own(rep, ty).try_into_resource_any(store)?,
    ))
}

/// `result<_, E>` that is `ok`.
fn ok_unit() -> Val {
    Val::Result(Ok(None))
}

/// Run the function `function` of [`TYPES`] with `args`, and answer its
/// one result, if it has one.
fn call_types<T: HttpHost>(
    call: &mut HostCall<'_, T>,
    function: &str,
    args: &[Val],
) -> Result<Option<Val>, Error> {
    let bad = || malformed(function, args);
    // Every function of the subset that takes a handle takes it first.
    let handle = rep_of(call.store(), args.first())?;
    Ok(Some(match function {
        "[constructor]fields" => {
            let table = call.data_mut().http();
            let rep = table.rep();
            table.fields.insert(rep, Fields::default());
            let types = table.types()?;
            new_handle(call.store(), rep, types.fields)?
        }
        "[static]fields.from-list" => {
            let [Val::List(entries)] = args else {
                return Err(bad());
            };
            let mut fields = Fields::default();
            for entry in entries.iter() {
                let Val::Tuple(pair) = entry else {
                    return Err(bad());
                };
                let [Val::String(name), value] = &pair[..] else {
                    return Err(bad());
                };
                fields
                    .entries
                    .push((name.clone(), bytes_of(value).ok_or_else(bad)?));
            }
            let table = call.data_mut().http();
            let rep = table.rep();
            table.fields.insert(rep, fields);
            let types = table.types()?;
            Val::Result(Ok(Some(Box::new(new_handle(
                call.store(),
                rep,
                types.fields,
            )?))))
        }
        "[method]fields.get" => {
            let (fields, name) = fields_and_name(call, handle, args).ok_or_else(bad)?;
            Val::List(
                fields
                    .entries
                    .iter()
                    .filter(|(field, _)| field.eq_ignore_ascii_case(&name))
                    .map(|(_, value)| bytes_val(value))
                    .collect(),
            )
        }
        "[method]fields.has" => {
            let (fields, name) = fields_and_name(call, handle, args).ok_or_else(bad)?;
            Val::Bool(
                fields
                    .entries
                    .iter()
                    .any(|(field, _)| field.eq_ignore_ascii_case(&name)),
            )
        }
        "[method]fields.copy-all" => {
            let rep = handle.ok_or_else(bad)?;
            let fields = call
                .data_mut()
                .http()
                .fields
                .get(&rep)
                .cloned()
                .ok_or_else(bad)?;
            Val::List(
                fields
                    .entries
                    .iter()
                    .map(|(name, value)| {
                        Val::Tuple(Box::new([Val::String(name.clone()), bytes_val(value)]))
                    })
                    .collect(),
            )
        }
        "[method]fields.append" | "[method]fields.set" | "[method]fields.delete" => {
            let rep = handle.ok_or_else(bad)?;
            let table = call.data_mut().http();
            let fields = table.fields.get_mut(&rep).ok_or_else(bad)?;
            if fields.immutable {
                return Ok(Some(header_error("immutable")));
            }
            let Some(Val::String(name)) = args.get(1) else {
                return Err(bad());
            };
            let name = name.to_ascii_lowercase();
            match function {
                "[method]fields.append" => {
                    let value = args.get(2).and_then(bytes_of).ok_or_else(bad)?;
                    fields.entries.push((name, value));
                }
                "[method]fields.set" => {
                    let Some(Val::List(values)) = args.get(2) else {
                        return Err(bad());
                    };
                    fields
                        .entries
                        .retain(|(field, _)| !field.eq_ignore_ascii_case(&name));
                    for value in values.iter() {
                        fields
                            .entries
                            .push((name.clone(), bytes_of(value).ok_or_else(bad)?));
                    }
                }
                _ => fields
                    .entries
                    .retain(|(field, _)| !field.eq_ignore_ascii_case(&name)),
            }
            ok_unit()
        }
        "[method]fields.clone" => {
            let rep = handle.ok_or_else(bad)?;
            let table = call.data_mut().http();
            let mut fields = table.fields.get(&rep).cloned().ok_or_else(bad)?;
            fields.immutable = false;
            let clone = table.rep();
            table.fields.insert(clone, fields);
            let types = table.types()?;
            new_handle(call.store(), clone, types.fields)?
        }
        "[static]request.new" => {
            let [_headers, contents, trailers, _options] = args else {
                return Err(bad());
            };
            discard::<TrailersOutcome, T>(call.store(), trailers)?;
            let headers_rep = handle.ok_or_else(bad)?;
            let body = match contents {
                Val::Option(None) => Body::Empty,
                Val::Option(Some(stream)) => Body::Stream(stream.as_ref().clone()),
                _ => return Err(bad()),
            };
            let table = call.data_mut().http();
            let mut headers = table.fields.remove(&headers_rep).ok_or_else(bad)?;
            headers.immutable = true;
            let rep = table.insert_request(Request {
                method: "GET".to_string(),
                scheme: None,
                authority: None,
                path_with_query: None,
                headers,
                body,
            });
            let types = table.types()?;
            let request = new_handle(call.store(), rep, types.request)?;
            let transmitted = FutureReader::new(call.store(), async {
                Ok::<_, Error>(Ok::<(), ErrorCode>(()))
            })?;
            let transmitted = FutureAny::try_from_future_reader(call.store(), transmitted)?;
            Val::Tuple(Box::new([request, Val::Future(transmitted)]))
        }
        "[method]request.get-method" => {
            let request = request(call, handle).ok_or_else(bad)?;
            method_val(&request.method)
        }
        "[method]request.set-method" => {
            let method = args.get(1).and_then(method_of).ok_or_else(bad)?;
            request_mut(call, handle).ok_or_else(bad)?.method = method;
            ok_unit()
        }
        "[method]request.get-path-with-query" => option_string(
            request(call, handle)
                .ok_or_else(bad)?
                .path_with_query
                .clone(),
        ),
        "[method]request.set-path-with-query" => {
            let value = args.get(1).and_then(string_option).ok_or_else(bad)?;
            request_mut(call, handle).ok_or_else(bad)?.path_with_query = value;
            ok_unit()
        }
        "[method]request.get-authority" => {
            option_string(request(call, handle).ok_or_else(bad)?.authority.clone())
        }
        "[method]request.set-authority" => {
            let value = args.get(1).and_then(string_option).ok_or_else(bad)?;
            request_mut(call, handle).ok_or_else(bad)?.authority = value;
            ok_unit()
        }
        "[method]request.get-scheme" => {
            let scheme = request(call, handle).ok_or_else(bad)?.scheme.clone();
            Val::Option(scheme.map(|scheme| Box::new(scheme_val(&scheme))))
        }
        "[method]request.set-scheme" => {
            let value = match args.get(1) {
                Some(Val::Option(None)) => None,
                Some(Val::Option(Some(scheme))) => Some(scheme_of(scheme).ok_or_else(bad)?),
                _ => return Err(bad()),
            };
            request_mut(call, handle).ok_or_else(bad)?.scheme = value;
            ok_unit()
        }
        "[method]request.get-headers" => {
            let headers = request(call, handle).ok_or_else(bad)?.headers.clone();
            new_fields(call, headers)?
        }
        "[method]response.get-headers" => {
            let headers = response(call, handle).ok_or_else(bad)?.headers.clone();
            new_fields(call, headers)?
        }
        "[method]request.get-options" => Val::Option(None),
        "[static]response.new" => {
            let [_headers, contents, trailers] = args else {
                return Err(bad());
            };
            discard::<TrailersOutcome, T>(call.store(), trailers)?;
            let headers_rep = handle.ok_or_else(bad)?;
            let body = match contents {
                Val::Option(None) => Body::Empty,
                Val::Option(Some(stream)) => Body::Stream(stream.as_ref().clone()),
                _ => return Err(bad()),
            };
            let table = call.data_mut().http();
            let mut headers = table.fields.remove(&headers_rep).ok_or_else(bad)?;
            headers.immutable = true;
            let rep = table.insert_response(Response {
                status: 200,
                headers,
                body,
            });
            let types = table.types()?;
            let response = new_handle(call.store(), rep, types.response)?;
            let transmitted = FutureReader::new(call.store(), async {
                Ok::<_, Error>(Ok::<(), ErrorCode>(()))
            })?;
            let transmitted = FutureAny::try_from_future_reader(call.store(), transmitted)?;
            Val::Tuple(Box::new([response, Val::Future(transmitted)]))
        }
        "[method]response.get-status-code" => {
            Val::U16(response(call, handle).ok_or_else(bad)?.status)
        }
        "[method]response.set-status-code" => {
            let Some(Val::U16(status)) = args.get(1) else {
                return Err(bad());
            };
            let status = *status;
            let rep = handle.ok_or_else(bad)?;
            call.data_mut()
                .http()
                .responses
                .get_mut(&rep)
                .ok_or_else(bad)?
                .status = status;
            ok_unit()
        }
        "[static]request.consume-body" | "[static]response.consume-body" => {
            let rep = handle.ok_or_else(bad)?;
            // How the guest's receipt of the body went: read, and kept
            // nowhere.
            discard::<Result<(), ErrorCode>, T>(call.store(), args.get(1).ok_or_else(bad)?)?;
            let table = call.data_mut().http();
            let body = if function == "[static]request.consume-body" {
                table.requests.remove(&rep).map(|request| request.body)
            } else {
                table.responses.remove(&rep).map(|response| response.body)
            }
            .ok_or_else(bad)?;
            let stream = match body {
                Body::Stream(stream) => stream,
                // A vector delivers its bytes on its first read and ends
                // the stream.
                Body::Empty => host_stream(call.store(), Vec::new())?,
                Body::Bytes(bytes) => host_stream(call.store(), bytes)?,
            };
            let trailers = FutureReader::new(call.store(), async {
                Ok::<_, Error>(TrailersOutcome::Ok(None))
            })?;
            let trailers = FutureAny::try_from_future_reader(call.store(), trailers)?;
            Val::Tuple(Box::new([stream, Val::Future(trailers)]))
        }
        _ => return Err(unsupported(function)),
    }))
}

/// A stream the host writes `bytes` to and then ends, as the untyped
/// value a host function answers.
fn host_stream<T: 'static>(store: &mut StoreContext<'_, T>, bytes: Vec<u8>) -> Result<Val, Error> {
    let reader = StreamReader::new(store, bytes)?;
    Ok(Val::Stream(StreamAny::try_from_stream_reader(
        store, reader,
    )?))
}

/// The fields of the handle `rep`, which `args[0]` held, and the field
/// name in `args[1]`, lower case.
fn fields_and_name<T: HttpHost>(
    call: &mut HostCall<'_, T>,
    rep: Option<u32>,
    args: &[Val],
) -> Option<(Fields, String)> {
    let rep = rep?;
    let Some(Val::String(name)) = args.get(1) else {
        return None;
    };
    let fields = call.data_mut().http().fields.get(&rep)?.clone();
    Some((fields, name.to_ascii_lowercase()))
}

/// The request of the handle `rep`.
fn request<'c, T: HttpHost>(
    call: &'c mut HostCall<'_, T>,
    rep: Option<u32>,
) -> Option<&'c Request> {
    call.data_mut().http().requests.get(&rep?)
}

/// The request of the handle `rep`, mutably.
fn request_mut<'c, T: HttpHost>(
    call: &'c mut HostCall<'_, T>,
    rep: Option<u32>,
) -> Option<&'c mut Request> {
    call.data_mut().http().requests.get_mut(&rep?)
}

/// The response of the handle `rep`.
fn response<'c, T: HttpHost>(
    call: &'c mut HostCall<'_, T>,
    rep: Option<u32>,
) -> Option<&'c Response> {
    call.data_mut().http().responses.get(&rep?)
}

/// A new immutable `fields` resource that holds `fields`.
fn new_fields<T: HttpHost>(call: &mut HostCall<'_, T>, mut fields: Fields) -> Result<Val, Error> {
    fields.immutable = true;
    let table = call.data_mut().http();
    let rep = table.rep();
    table.fields.insert(rep, fields);
    let types = table.types()?;
    new_handle(call.store(), rep, types.fields)
}

/// `err` with the `header-error` case `case`.
fn header_error(case: &str) -> Val {
    Val::Result(Err(Some(Box::new(Val::Variant {
        discriminant: case.to_string(),
        payload: None,
    }))))
}

/// The bytes of a `list<u8>`.
fn bytes_of(val: &Val) -> Option<Vec<u8>> {
    let Val::List(items) = val else {
        return None;
    };
    items
        .iter()
        .map(|item| match item {
            Val::U8(byte) => Some(*byte),
            _ => None,
        })
        .collect()
}

/// A `list<u8>` of `bytes`.
fn bytes_val(bytes: &[u8]) -> Val {
    Val::List(bytes.iter().map(|byte| Val::U8(*byte)).collect())
}

/// `option<string>`.
fn option_string(value: Option<String>) -> Val {
    Val::Option(value.map(|value| Box::new(Val::String(value))))
}

/// The value of an `option<string>`.
fn string_option(val: &Val) -> Option<Option<String>> {
    match val {
        Val::Option(None) => Some(None),
        Val::Option(Some(text)) => match text.as_ref() {
            Val::String(text) => Some(Some(text.clone())),
            _ => None,
        },
        _ => None,
    }
}

/// The cases of `method`, in the order WIT declares them, and the
/// method each names.
const METHODS: [(&str, &str); 9] = [
    ("get", "GET"),
    ("head", "HEAD"),
    ("post", "POST"),
    ("put", "PUT"),
    ("delete", "DELETE"),
    ("connect", "CONNECT"),
    ("options", "OPTIONS"),
    ("trace", "TRACE"),
    ("patch", "PATCH"),
];

/// The `method` variant of `method`.
fn method_val(method: &str) -> Val {
    match METHODS.iter().find(|(_, name)| *name == method) {
        Some((case, _)) => Val::Variant {
            discriminant: (*case).to_string(),
            payload: None,
        },
        None => Val::Variant {
            discriminant: "other".to_string(),
            payload: Some(Box::new(Val::String(method.to_string()))),
        },
    }
}

/// The method a `method` variant names.
fn method_of(val: &Val) -> Option<String> {
    let Val::Variant {
        discriminant,
        payload,
    } = val
    else {
        return None;
    };
    if discriminant == "other" {
        return match payload.as_deref() {
            Some(Val::String(name)) => Some(name.clone()),
            _ => None,
        };
    }
    METHODS
        .iter()
        .find(|(case, _)| case == discriminant)
        .map(|(_, name)| (*name).to_string())
}

/// The `scheme` variant of `scheme`.
fn scheme_val(scheme: &str) -> Val {
    match scheme {
        "http" => Val::Variant {
            discriminant: "HTTP".to_string(),
            payload: None,
        },
        "https" => Val::Variant {
            discriminant: "HTTPS".to_string(),
            payload: None,
        },
        other => Val::Variant {
            discriminant: "other".to_string(),
            payload: Some(Box::new(Val::String(other.to_string()))),
        },
    }
}

/// The scheme a `scheme` variant names.
fn scheme_of(val: &Val) -> Option<String> {
    let Val::Variant {
        discriminant,
        payload,
    } = val
    else {
        return None;
    };
    match (discriminant.as_str(), payload.as_deref()) {
        ("HTTP", None) => Some("http".to_string()),
        ("HTTPS", None) => Some("https".to_string()),
        ("other", Some(Val::String(scheme))) => Some(scheme.clone()),
        _ => None,
    }
}

/// The bytes a guest wrote to a body stream, and whether the stream has
/// ended.
#[derive(Default)]
struct Collected {
    bytes: Vec<u8>,
    ended: bool,
    waker: Option<Waker>,
}

/// A consumer that collects every byte of a body stream.
struct Collects(Arc<Mutex<Collected>>);

impl<T: 'static> StreamConsumer<T> for Collects {
    type Item = u8;

    fn poll_consume(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        store: &mut StoreContext<'_, T>,
        mut source: Source<'_, u8>,
        _finish: bool,
    ) -> Poll<Result<StreamResult, Error>> {
        let mut bytes = Vec::new();
        let count = source.remaining();
        source.read(store, &mut bytes, count)?;
        lock(&self.0).bytes.extend(bytes);
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

impl Drop for Collects {
    /// The store drops the consumer once the guest drops the stream's
    /// writable end: the body is whole.
    fn drop(&mut self) {
        let waker = {
            let mut collected = lock(&self.0);
            collected.ended = true;
            collected.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// A consumer that passes each piece of a body stream on as the guest
/// writes it. The channel closes when the guest drops the stream's
/// writable end: the body has ended.
struct Passes(futures::channel::mpsc::UnboundedSender<Vec<u8>>);

impl<T: 'static> StreamConsumer<T> for Passes {
    type Item = u8;

    fn poll_consume(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        store: &mut StoreContext<'_, T>,
        mut source: Source<'_, u8>,
        _finish: bool,
    ) -> Poll<Result<StreamResult, Error>> {
        let mut bytes = Vec::new();
        let count = source.remaining();
        source.read(store, &mut bytes, count)?;
        if bytes.is_empty() {
            return Poll::Ready(Ok(StreamResult::Completed));
        }
        // A reader that went away wants no more of the body.
        match self.0.unbounded_send(bytes) {
            Ok(()) => Poll::Ready(Ok(StreamResult::Completed)),
            Err(_) => Poll::Ready(Ok(StreamResult::Dropped)),
        }
    }
}

/// Start reading `body` in `store`, and answer each piece of it as the
/// guest writes it, for as long as the store is driven. A body the host
/// holds is one piece.
pub fn stream_body<T: 'static>(
    store: &mut StoreContext<'_, T>,
    body: Body,
) -> Result<futures::channel::mpsc::UnboundedReceiver<Vec<u8>>, Error> {
    let (pieces, read) = futures::channel::mpsc::unbounded();
    match body {
        Body::Empty => {}
        Body::Bytes(bytes) => {
            let _ = pieces.unbounded_send(bytes);
        }
        Body::Stream(stream) => {
            let Val::Stream(stream) = stream else {
                return Err(Error::Internal {
                    message: "a body is a `stream<u8>`".to_string(),
                });
            };
            stream
                .try_into_stream_reader::<u8>()?
                .pipe(store, Passes(pieces))?;
        }
    }
    Ok(read)
}

/// Lock `shared`, whatever a panic elsewhere left in it.
fn lock<T>(shared: &Mutex<T>) -> MutexGuard<'_, T> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Start reading `body` in `store`, and answer a future of its bytes,
/// which resolves once the guest has written all of them. A body the
/// host holds resolves at once.
///
/// The future resolves only while a driver runs turns of the store,
/// such as the `run_concurrent` around the call that made the body.
///
/// # Errors
///
/// The polyfill's error when the stream cannot be read.
pub fn read_body<T: 'static>(
    store: &mut StoreContext<'_, T>,
    body: Body,
) -> Result<impl core::future::Future<Output = Vec<u8>> + use<T>, Error> {
    let collected = Arc::new(Mutex::new(Collected::default()));
    match body {
        Body::Empty => lock(&collected).ended = true,
        Body::Bytes(bytes) => {
            let mut collected = lock(&collected);
            collected.bytes = bytes;
            collected.ended = true;
        }
        Body::Stream(stream) => {
            let Val::Stream(stream) = stream else {
                return Err(Error::Internal {
                    message: "a body is a `stream<u8>`".to_string(),
                });
            };
            stream
                .try_into_stream_reader::<u8>()?
                .pipe(store, Collects(collected.clone()))?;
        }
    }
    Ok(core::future::poll_fn(move |cx| {
        let mut collected = lock(&collected);
        if collected.ended {
            Poll::Ready(core::mem::take(&mut collected.bytes))
        } else {
            collected.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }))
}

/// A consumer that reads the one value of a future a guest handed the
/// host, and keeps none of it.
///
/// A guest that hands the host the readable end of a future, such as a
/// request's trailers, writes the value and waits for that write to
/// complete before its task can end. A host that only dropped the end
/// would leave the write, and with it the task, waiting forever. So the
/// host reads each such future, even where the subset has no use for
/// its value.
struct Discards<T>(core::marker::PhantomData<fn() -> T>);

impl<T: Lift + 'static, D: 'static> FutureConsumer<D> for Discards<T> {
    type Item = T;

    fn poll_consume(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        store: &mut StoreContext<'_, D>,
        mut source: Source<'_, T>,
        _finish: bool,
    ) -> Poll<Result<(), Error>> {
        let mut values = Vec::new();
        source.read(store, &mut values, 1)?;
        Poll::Ready(Ok(()))
    }
}

/// Read and discard the future `val` a guest handed the host, whose
/// payload is `T`.
fn discard<T: Lift + 'static, D: 'static>(
    store: &mut StoreContext<'_, D>,
    val: &Val,
) -> Result<(), Error> {
    let Val::Future(future) = val else {
        return Err(Error::Internal {
            message: format!("not a future: {val:?}"),
        });
    };
    future
        .clone()
        .try_into_future_reader::<T>()?
        .pipe(store, Discards::<T>(core::marker::PhantomData))
}

/// The payload of a trailers future the host resolves or reads:
/// `result<option<trailers>, error-code>`.
///
/// The trailers are an `own<fields>`, a dynamic host resource, so they
/// cross as a [`ResourceDynamic`]. The host only ever writes
/// `ok(none)`. It reads trailers a guest writes only to let the
/// guest's write complete, and drops them unread. The `err` crosses
/// whole, payload and all.
type TrailersOutcome = Result<Option<ResourceDynamic>, ErrorCode>;

/// `wasi:http/types.error-code`, as `wasi:http@0.3.0` declares it,
/// case by case and in order.
#[derive(ComponentType, Lift, Lower)]
#[component(variant)]
enum ErrorCode {
    #[component(name = "DNS-timeout")]
    DnsTimeout,
    #[component(name = "DNS-error")]
    DnsError(DnsErrorPayload),
    #[component(name = "destination-not-found")]
    DestinationNotFound,
    #[component(name = "destination-unavailable")]
    DestinationUnavailable,
    #[component(name = "destination-IP-prohibited")]
    DestinationIpProhibited,
    #[component(name = "destination-IP-unroutable")]
    DestinationIpUnroutable,
    #[component(name = "connection-refused")]
    ConnectionRefused,
    #[component(name = "connection-terminated")]
    ConnectionTerminated,
    #[component(name = "connection-timeout")]
    ConnectionTimeout,
    #[component(name = "connection-read-timeout")]
    ConnectionReadTimeout,
    #[component(name = "connection-write-timeout")]
    ConnectionWriteTimeout,
    #[component(name = "connection-limit-reached")]
    ConnectionLimitReached,
    #[component(name = "TLS-protocol-error")]
    TlsProtocolError,
    #[component(name = "TLS-certificate-error")]
    TlsCertificateError,
    #[component(name = "TLS-alert-received")]
    TlsAlertReceived(TlsAlertReceivedPayload),
    #[component(name = "HTTP-request-denied")]
    HttpRequestDenied,
    #[component(name = "HTTP-request-length-required")]
    HttpRequestLengthRequired,
    #[component(name = "HTTP-request-body-size")]
    HttpRequestBodySize(Option<u64>),
    #[component(name = "HTTP-request-method-invalid")]
    HttpRequestMethodInvalid,
    #[component(name = "HTTP-request-URI-invalid")]
    HttpRequestUriInvalid,
    #[component(name = "HTTP-request-URI-too-long")]
    HttpRequestUriTooLong,
    #[component(name = "HTTP-request-header-section-size")]
    HttpRequestHeaderSectionSize(Option<u32>),
    #[component(name = "HTTP-request-header-size")]
    HttpRequestHeaderSize(Option<FieldSizePayload>),
    #[component(name = "HTTP-request-trailer-section-size")]
    HttpRequestTrailerSectionSize(Option<u32>),
    #[component(name = "HTTP-request-trailer-size")]
    HttpRequestTrailerSize(FieldSizePayload),
    #[component(name = "HTTP-response-incomplete")]
    HttpResponseIncomplete,
    #[component(name = "HTTP-response-header-section-size")]
    HttpResponseHeaderSectionSize(Option<u32>),
    #[component(name = "HTTP-response-header-size")]
    HttpResponseHeaderSize(FieldSizePayload),
    #[component(name = "HTTP-response-body-size")]
    HttpResponseBodySize(Option<u64>),
    #[component(name = "HTTP-response-trailer-section-size")]
    HttpResponseTrailerSectionSize(Option<u32>),
    #[component(name = "HTTP-response-trailer-size")]
    HttpResponseTrailerSize(FieldSizePayload),
    #[component(name = "HTTP-response-transfer-coding")]
    HttpResponseTransferCoding(Option<String>),
    #[component(name = "HTTP-response-content-coding")]
    HttpResponseContentCoding(Option<String>),
    #[component(name = "HTTP-response-timeout")]
    HttpResponseTimeout,
    #[component(name = "HTTP-upgrade-failed")]
    HttpUpgradeFailed,
    #[component(name = "HTTP-protocol-error")]
    HttpProtocolError,
    #[component(name = "loop-detected")]
    LoopDetected,
    #[component(name = "configuration-error")]
    ConfigurationError,
    #[component(name = "internal-error")]
    InternalError(Option<String>),
}

/// The payload of [`ErrorCode::DnsError`].
#[derive(ComponentType, Lift, Lower)]
#[component(record)]
struct DnsErrorPayload {
    rcode: Option<String>,
    #[component(name = "info-code")]
    info_code: Option<u16>,
}

/// The payload of [`ErrorCode::TlsAlertReceived`].
#[derive(ComponentType, Lift, Lower)]
#[component(record)]
struct TlsAlertReceivedPayload {
    #[component(name = "alert-id")]
    alert_id: Option<u8>,
    #[component(name = "alert-message")]
    alert_message: Option<String>,
}

/// The payload of the error codes about the size of one field.
#[derive(ComponentType, Lift, Lower)]
#[component(record)]
struct FieldSizePayload {
    #[component(name = "field-name")]
    field_name: Option<String>,
    #[component(name = "field-size")]
    field_size: Option<u32>,
}
