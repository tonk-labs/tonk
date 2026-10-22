// Copyright 2026 The Dialog DB Project
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The few WASI functions the demo's components call, defined on a
//! polyfill [`Linker`].
//!
//! The demo is not a WASI host. It supplies only what Zena's output
//! imports on the component target, at the versions Zena emits:
//!
//! - `wasi:cli/stdout@0.3.0` and `wasi:cli/stderr@0.3.0`, whose
//!   `write-via-stream` takes a `stream<u8>` and answers a future that
//!   resolves when the guest drops the stream. Each line the guest
//!   writes goes to the console, under a label that names the
//!   component: `console.log` for standard output and `console.error`
//!   for standard error in the browser. `wasi:cli/types@0.3.0` holds no
//!   function.
//! - `wasi:clocks/monotonic-clock@0.3.0`: `now` reads the clock of the
//!   context, `performance.now()` in the browser, and `wait-for` waits
//!   on its timer, a `setTimeout`. `wasi:clocks/types@0.3.0` holds no
//!   function.
//!
//! A function of these interfaces that Zena's output does not call
//! returns an error when it is called, so a change in Zena that starts
//! calling one shows as a failure, not a silent wrong answer.

use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex, MutexGuard};

use wcmp::{
    Accessor, ComponentType, EnumType, Error, FunctionParameter, FunctionType, FutureAny,
    FutureProducer, FutureReader, FutureType, HostCall, InterfaceIdentifier, Lift, Linker, Lower,
    PrimitiveType, ResultType, Source, StoreContext, StreamConsumer, StreamReader, StreamResult,
    StreamType, Val, ValueType,
};

use crate::platform;

/// The interface `wasi:cli/stdout` and `wasi:cli/stderr` import their
/// `error-code` from.
const CLI_TYPES: &str = "wasi:cli/types@0.3.0";

/// The interface a component prints through.
const STDOUT: &str = "wasi:cli/stdout@0.3.0";

/// The interface a component writes its error lines through.
const STDERR: &str = "wasi:cli/stderr@0.3.0";

/// The interface `wasi:clocks/monotonic-clock` imports its `duration`
/// from.
const CLOCK_TYPES: &str = "wasi:clocks/types@0.3.0";

/// The interface a component reads the time and sleeps through.
const MONOTONIC_CLOCK: &str = "wasi:clocks/monotonic-clock@0.3.0";

/// The cases of `wasi:cli/types.error-code`, in order.
const ERROR_CODES: [&str; 3] = ["io", "illegal-byte-sequence", "pipe"];

/// Define the WASI functions in `linker`. Each line a component prints
/// goes to the console after `label`, which names the component.
///
/// # Errors
///
/// The polyfill's error when it refuses a definition.
pub fn define<T: 'static>(linker: &mut Linker<T>, label: &str) -> Result<(), Error> {
    linker.instance(&identifier(CLI_TYPES));
    define_output(linker, STDOUT, label, Console::Log)?;
    define_output(linker, STDERR, label, Console::Error)?;
    linker.instance(&identifier(CLOCK_TYPES));
    let mut monotonic_clock = linker.instance(&identifier(MONOTONIC_CLOCK));
    monotonic_clock.func_wrap("now", |_, (): ()| Ok((platform::now_nanos(),)))?;
    monotonic_clock.func_wrap("get-resolution", |_, (): ()| {
        Err::<(u64,), _>(unimplemented(MONOTONIC_CLOCK, "get-resolution"))
    })?;
    monotonic_clock.func_wrap_concurrent("wait-for", |_: &Accessor<T>, (how_long,): (u64,)| {
        platform::wait(how_long)
    })?;
    monotonic_clock.func_wrap_concurrent("wait-until", |_: &Accessor<T>, (_when,): (u64,)| {
        core::future::ready(Err::<(), _>(unimplemented(MONOTONIC_CLOCK, "wait-until")))
    })?;
    Ok(())
}

/// Where the lines of one output go.
#[derive(Clone, Copy)]
enum Console {
    /// `console.log`.
    Log,
    /// `console.error`.
    Error,
}

impl Console {
    /// Write `line` after `label`.
    fn write(self, label: &str, line: &str) {
        platform::log(label, line, matches!(self, Console::Error));
    }
}

/// Define `write-via-stream` of the output interface `interface` in
/// `linker`. Each line the guest writes goes to `console`.
fn define_output<T: 'static>(
    linker: &mut Linker<T>,
    interface: &str,
    label: &str,
    console: Console,
) -> Result<(), Error> {
    let label = format!("[{label}]");
    linker.instance(&identifier(interface)).func_new(
        "write-via-stream",
        FunctionType {
            parameters: vec![FunctionParameter {
                name: "data".to_string(),
                ty: ValueType::Stream(StreamType::new(Some(ValueType::Primitive(
                    PrimitiveType::U8,
                )))),
            }],
            result: Some(ValueType::Future(FutureType::new(
                Some(write_result_type()),
            ))),
            async_: false,
        },
        move |mut call: HostCall<'_, T>, args, results| {
            let [Val::Stream(data)] = args else {
                return Err(Error::Internal {
                    message: format!("`write-via-stream` was given {args:?}"),
                });
            };
            let end = Arc::new(Mutex::new(End::default()));
            let consumer = Lines {
                label: label.clone(),
                console,
                pending: Vec::new(),
                end: end.clone(),
            };
            StreamReader::<u8>::try_from_stream_any(data.clone())?.pipe(call.store(), consumer)?;
            let finished = FutureReader::new(call.store(), Finishes(end))?;
            results[0] = Val::Future(FutureAny::try_from_future_reader(call.store(), finished)?);
            Ok(())
        },
    )
}

/// What a function of `interface` the demo does not implement,
/// `method`, answers when it is called.
fn unimplemented(interface: &str, method: &str) -> Error {
    Error::Unsupported {
        feature: format!("`{interface}#{method}` in the demo's WASI functions"),
    }
}

/// The identifier of an interface this file names.
fn identifier(interface: &str) -> InterfaceIdentifier {
    interface
        .parse()
        .unwrap_or_else(|error| panic!("`{interface}` is not an interface: {error}"))
}

/// The end of one stream passed to `write-via-stream`, shared by its
/// consumer and the future that reports it.
#[derive(Default)]
struct End {
    /// Whether the guest dropped the stream's writable end.
    ended: bool,
    /// The future waiting for that end.
    waker: Option<Waker>,
}

/// The consumer of a stream passed to `write-via-stream`: it writes
/// each complete line to the console, and the rest when the stream
/// ends.
struct Lines {
    label: String,
    console: Console,
    pending: Vec<u8>,
    end: Arc<Mutex<End>>,
}

impl Lines {
    /// Write every complete line of `pending` to the console.
    fn flush_lines(&mut self) {
        while let Some(newline) = self.pending.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=newline).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]);
            self.console.write(&self.label, &text);
        }
    }
}

impl<T: 'static> StreamConsumer<T> for Lines {
    type Item = u8;

    fn poll_consume(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        store: &mut StoreContext<'_, T>,
        mut source: Source<'_, u8>,
        _finish: bool,
    ) -> Poll<Result<StreamResult, Error>> {
        let mut bytes = Vec::new();
        let count = source.remaining();
        source.read(store, &mut bytes, count)?;
        self.pending.extend(bytes);
        self.flush_lines();
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

impl Drop for Lines {
    /// The store drops the consumer once the guest drops the stream's
    /// writable end: the last partial line goes out, and the future of
    /// the write resolves.
    fn drop(&mut self) {
        if !self.pending.is_empty() {
            let text = String::from_utf8_lossy(&self.pending).into_owned();
            self.console.write(&self.label, &text);
        }
        let waker = {
            let mut end = lock(&self.end);
            end.ended = true;
            end.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// The producer of the future `write-via-stream` answers: `ok` once its
/// stream has ended.
struct Finishes(Arc<Mutex<End>>);

impl<T: 'static> FutureProducer<T> for Finishes {
    type Item = WriteResult;

    fn poll_produce(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _store: &mut StoreContext<'_, T>,
        finish: bool,
    ) -> Poll<Result<Option<WriteResult>, Error>> {
        let mut end = lock(&self.0);
        if end.ended {
            return Poll::Ready(Ok(Some(Ok(()))));
        }
        if finish {
            return Poll::Ready(Ok(None));
        }
        end.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// The payload of the future `write-via-stream` answers:
/// `result<_, error-code>`. The demo never fails a write, so it only
/// ever holds `ok`.
type WriteResult = Result<(), ErrorCode>;

/// `wasi:cli/types.error-code`, the error of a write.
#[derive(ComponentType, Lift, Lower, Debug)]
#[component(enum)]
#[repr(u8)]
enum ErrorCode {
    #[component(name = "io")]
    Io,
    #[component(name = "illegal-byte-sequence")]
    IllegalByteSequence,
    #[component(name = "pipe")]
    Pipe,
}

/// The type `write-via-stream`'s future carries:
/// `result<_, error-code>`.
fn write_result_type() -> ValueType {
    let codes = EnumType::new(ERROR_CODES.map(str::to_string));
    ValueType::Result(ResultType::new(None, Some(ValueType::Enum(codes))))
}

/// Lock `shared`, whatever a panic elsewhere left in it.
fn lock<T>(shared: &Mutex<T>) -> MutexGuard<'_, T> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
