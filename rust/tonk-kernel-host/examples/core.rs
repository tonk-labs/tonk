//! Write out the largest core module inside a component: the kernel's own
//! code, without the component's adapters. For measuring how much of a
//! component's compile is the browser compiling that module.
fn main() {
    let mut args = std::env::args().skip(1);
    let (input, output) = (
        args.next().expect("a component"),
        args.next().expect("an output path"),
    );
    let bytes = std::fs::read(&input).expect("the component reads");
    let mut largest: &[u8] = &[];
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let Ok(wasmparser::Payload::ModuleSection {
            unchecked_range, ..
        }) = payload
        {
            let module = &bytes[unchecked_range.start as usize..unchecked_range.end as usize];
            if module.len() > largest.len() {
                largest = module;
            }
        }
    }
    std::fs::write(&output, largest).expect("the module writes");
    println!("{} bytes", largest.len());
    // Where the module's bytes are, by section.
    for payload in wasmparser::Parser::new(0).parse_all(largest) {
        match payload {
            Ok(wasmparser::Payload::CustomSection(section)) => {
                println!("  custom {:>12} {}", section.data().len(), section.name());
            }
            Ok(wasmparser::Payload::CodeSectionStart { size, count, .. }) => {
                println!("  code   {size:>12} ({count} functions)");
            }
            Ok(wasmparser::Payload::DataSection(reader)) => {
                println!("  data   {:>12}", reader.range().end - reader.range().start);
            }
            _ => {}
        }
    }
}
