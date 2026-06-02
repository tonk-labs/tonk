//! Compile-time macros for tonk.
//!
//! [`claim!`] reads a notation file at compile time, analyzes it
//! against its own definitions (no running system), lowers it to a
//! [`tonk_core::claim::TransactRequest`], and embeds the result.
//! See the macro's own docs for the contract.

use proc_macro::TokenStream;
use quote::quote;
use std::path::PathBuf;
use syn::{LitStr, parse_macro_input};

/// Compile a self-contained notation file into a
/// [`tonk_core::claim::TransactRequest`] at build time.
///
/// ```ignore
/// use std::sync::LazyLock;
/// use tonk_core::claim::TransactRequest;
///
/// static BOOTSTRAP: LazyLock<TransactRequest> =
///     LazyLock::new(|| tonk_notation::claim!("bootstrap.yaml"));
/// ```
///
/// The argument is a path relative to the calling crate's
/// `CARGO_MANIFEST_DIR` (same convention as `include_str!`). The
/// file is parsed and analyzed with no branch: every reference
/// must resolve against the document's own `concept!` /
/// `attribute!` / `&anchor` definitions (plus builtins). A
/// reference that would need a running system, a parse error, or
/// an analysis error all become compile errors.
///
/// The macro emits code that reconstructs the request at runtime
/// from its embedded canonical DAG-JSON bytes.
#[proc_macro]
pub fn claim(input: TokenStream) -> TokenStream {
    let lit = parse_macro_input!(input as LitStr);
    match build(&lit) {
        Ok(tokens) => tokens,
        Err(message) => syn::Error::new(lit.span(), message)
            .to_compile_error()
            .into(),
    }
}

fn build(lit: &LitStr) -> Result<TokenStream, String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| "claim!: CARGO_MANIFEST_DIR is not set".to_owned())?;
    let path = PathBuf::from(manifest).join(lit.value());

    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("claim!: cannot read {}: {e}", path.display()))?;

    let parsed = tonk_notation::parse(&text);
    let syntax = parsed.syntax.ok_or_else(|| {
        let detail = parsed
            .diagnostics
            .first()
            .map(|d| d.message.clone())
            .unwrap_or_else(|| "no parseable document".to_owned());
        format!("claim!: parse failed in {}: {detail}", path.display())
    })?;

    let tree = tonk_analyzer::analyzer::analyze_local(&syntax)
        .map_err(|e| format!("claim!: analysis failed in {}: {e}", path.display()))?;

    let request = tree
        .analysis
        .lower_to_claims()
        .map_err(|e| format!("claim!: lowering failed in {}: {e}", path.display()))?;

    let bytes = serde_ipld_dagjson::to_vec(&request)
        .map_err(|e| format!("claim!: serialization failed: {e}"))?;

    let byte_literal = proc_macro2::Literal::byte_string(&bytes);
    // Emit an `include_bytes!` of the source path so the COMPILER
    // records it as a build dependency. A proc-macro reading a file
    // with `std::fs` is invisible to cargo's dependency graph, so
    // editing the notation document would NOT trigger a rebuild and
    // the stale lowered bytes would keep being served. `include_bytes!`
    // makes the build correctly re-run when the document changes.
    let path_str = path.to_string_lossy();
    let expanded = quote! {
        {
            const _: &[u8] = include_bytes!(#path_str);
            ::tonk_core::claim::TransactRequest::from_dagjson_bytes(#byte_literal)
        }
    };
    Ok(expanded.into())
}
