//! `tonk-kernel-bundle <component> <recorded> <out>`: write the component a
//! host that compiles no WebAssembly is deployed with to `<out>/kernel.component`,
//! and each module it is deployed with to `<out>/modules/<key>.wasm`, from the
//! modules a recording run wrote to `<recorded>` by their keys. Each core module
//! of the component is filed under its skeleton's key. Fails when the
//! recording lacks a core module of the component.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [component, recorded, out] = arguments.as_slice() else {
        eprintln!("usage: tonk-kernel-bundle <component> <recorded> <out>");
        return ExitCode::FAILURE;
    };
    match bundle(Path::new(component), Path::new(recorded), Path::new(out)) {
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
    fs::create_dir_all(&modules).map_err(|error| error.to_string())?;
    fs::write(out.join("kernel.component"), &skeleton).map_err(|error| error.to_string())?;
    let entries =
        fs::read_dir(recorded).map_err(|error| format!("{}: {error}", recorded.display()))?;
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
            .map_or(key, |renamed| renamed.skeleton.as_str());
        fs::copy(&path, modules.join(format!("{filed}.wasm")))
            .map_err(|error| error.to_string())?;
    }
    for renamed in &renamed {
        if !modules.join(format!("{}.wasm", renamed.skeleton)).is_file() {
            return Err(format!(
                "the recording lacks the core module {}",
                renamed.module
            ));
        }
    }
    println!(
        "{} bytes of component, {} core modules filed under their skeletons",
        skeleton.len(),
        renamed.len()
    );
    Ok(())
}
