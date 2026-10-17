//! `#[wasm_compat]` for async axum handlers.
//!
//! Axum requires a handler's future to be `Send`.
//!
//! In the browser, JS values are not `Send`, so the handler body runs on the
//! local executor and the handler awaits its result over a channel, as
//! `axum-wasm-macros` does. That expansion is unchanged.
//!
//! Natively the body is boxed as a `dyn Future + Send`. The body is proven
//! `Send` once, where it is boxed. Without the box, the handler's future is
//! the body's whole state machine, including every future it awaits, and
//! rustc proves it `Send` again for each trait query axum's routing makes
//! of it during code generation. For tonk-worker's handlers that was most of
//! the crate's native compile time.
//!
//! ```ignore
//! #[wasm_compat]
//! pub async fn index() -> &'static str {
//!     "Hello World"
//! }
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, ReturnType, parse_macro_input};

#[proc_macro_attribute]
pub fn wasm_compat(_attr: TokenStream, stream: TokenStream) -> TokenStream {
    let ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = parse_macro_input!(stream as ItemFn);
    let stmts = &block.stmts;
    let output = match &sig.output {
        ReturnType::Default => quote! { () },
        ReturnType::Type(_, ty) => quote! { #ty },
    };

    quote! {
        #[cfg(target_arch = "wasm32")]
        #(#attrs)* #vis #sig {
            let (tx, rx) = oneshot::channel();
            wasm_bindgen_futures::spawn_local(async move {
                let result = (|| async move {
                    #(#stmts)*
                })().await;

                tx.send(result).unwrap();
            });

            rx.await.expect("Failed to send result")
        }

        #[cfg(not(target_arch = "wasm32"))]
        #(#attrs)* #vis #sig {
            let future: ::std::pin::Pin<
                ::std::boxed::Box<
                    dyn ::std::future::Future<Output = #output> + ::std::marker::Send,
                >,
            > = ::std::boxed::Box::pin(async move { #(#stmts)* });
            future.await
        }
    }
    .into()
}
