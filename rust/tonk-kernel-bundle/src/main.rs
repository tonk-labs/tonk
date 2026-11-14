//! The kernel a Worker is deployed with.
//!
//! - `tonk-kernel-bundle component <out>`: write the space's kernel this
//!   build ships (`tonk_components::TONK_KERNEL_BYTES`) to `<out>`, for a
//!   recording run to boot.
//! - `tonk-kernel-bundle bundle <component> <recorded> <out>`: write the
//!   component a host that compiles no WebAssembly is deployed with to
//!   `<out>/kernel.component`, each module it is deployed with to
//!   `<out>/modules/<key>.wasm` (from the modules a recording run wrote to
//!   `<recorded>` by their keys, each core module of the component filed under
//!   its skeleton's key), and `<out>/modules/index.mjs`, which imports them
//!   all as `[key, module]` pairs. Fails when the recording lacks a core
//!   module of the component.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let done = match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["component", out] => fs::write(out, tonk_components::TONK_KERNEL_BYTES)
            .map_err(|error| format!("{out}: {error}")),
        ["bundle", component, recorded, out] => {
            bundle(Path::new(component), Path::new(recorded), Path::new(out))
        }
        _ => Err(
            "usage: tonk-kernel-bundle component <out> | bundle <component> <recorded> <out>"
                .into(),
        ),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tonk-kernel-bundle: {error}");
            ExitCode::FAILURE
        }
    }
}

fn bundle(component: &Path, recorded: &Path, out: &Path) -> Result<(), String> {
    let read = |path: &Path| fs::read(path).map_err(|error| format!("{}: {error}", path.display()));
    let (skeleton, renamed) = tonk_kernel_bundle::skeleton(&read(component)?)?;
    let modules = out.join("modules");
    if modules.exists() {
        fs::remove_dir_all(&modules).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&modules).map_err(|error| error.to_string())?;
    fs::write(out.join("kernel.component"), &skeleton).map_err(|error| error.to_string())?;
    let entries =
        fs::read_dir(recorded).map_err(|error| format!("{}: {error}", recorded.display()))?;
    let mut keys = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().is_none_or(|extension| extension != "wasm") {
            continue;
        }
        let Some(key) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let filed = renamed
            .iter()
            .find(|renamed| renamed.module == key)
            .map_or(key, |renamed| renamed.skeleton.as_str())
            .to_owned();
        fs::copy(&path, modules.join(format!("{filed}.wasm")))
            .map_err(|error| error.to_string())?;
        keys.push(filed);
    }
    for renamed in &renamed {
        if !keys.contains(&renamed.skeleton) {
            return Err(format!(
                "the recording lacks the core module {}",
                renamed.module
            ));
        }
    }
    keys.sort();
    let mut index = String::new();
    for (at, key) in keys.iter().enumerate() {
        index.push_str(&format!("import m{at} from \"./{key}.wasm\";\n"));
    }
    let pairs: Vec<String> = keys
        .iter()
        .enumerate()
        .map(|(at, key)| format!("[\"{key}\", m{at}]"))
        .collect();
    index.push_str(&format!("export const modules = [{}];\n", pairs.join(", ")));
    fs::write(modules.join("index.mjs"), index).map_err(|error| error.to_string())?;
    println!(
        "{} bytes of component, {} modules, {} core modules filed under their skeletons",
        skeleton.len(),
        keys.len(),
        renamed.len()
    );
    Ok(())
}
