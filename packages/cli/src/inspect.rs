use anyhow::{Context, Result};
use base64::Engine;
use std::fs;
use std::path::Path;

/// Remove a field from a CBOR file
pub fn remove_field(file_path: &str, field_name: &str) -> Result<()> {
    let data = fs::read(file_path).context(format!("Failed to read file: {}", file_path))?;

    // Decode CBOR
    let mut value: ciborium::Value =
        ciborium::from_reader(&data[..]).context("Failed to decode CBOR")?;

    // Remove field
    if let ciborium::Value::Map(ref mut map) = value {
        let original_len = map.len();
        map.retain(|(k, _)| {
            if let ciborium::Value::Text(key) = k {
                key != field_name
            } else {
                true
            }
        });

        if map.len() < original_len {
            // Write back
            let mut output = Vec::new();
            ciborium::into_writer(&value, &mut output).context("Failed to encode CBOR")?;
            fs::write(file_path, output).context("Failed to write file")?;
            println!("Removed '{}' field and saved file", field_name);
        } else {
            println!("No '{}' field found", field_name);
        }
    } else {
        anyhow::bail!("CBOR root is not a map");
    }

    Ok(())
}

/// Inspect CBOR data and display as JSON
pub fn cbor(input: String) -> Result<()> {
    let bytes = if Path::new(&input).exists() {
        // Read from file
        fs::read(&input).context(format!("Failed to read file: {}", input))?
    } else {
        // Try to decode as base64
        base64::engine::general_purpose::STANDARD
            .decode(&input)
            .or_else(|_| {
                // Try URL-safe base64
                base64::engine::general_purpose::URL_SAFE
                    .decode(&input)
            })
            .or_else(|_| {
                // Try base64 without padding
                base64::engine::general_purpose::STANDARD_NO_PAD
                    .decode(&input)
            })
            .context("Input is not a valid file path or base64-encoded data")?
    };

    // Decode CBOR to a generic value
    let value: ciborium::Value = ciborium::from_reader(&bytes[..])
        .context("Failed to decode CBOR data")?;

    // Convert to JSON
    let json = cbor_value_to_json(&value);

    // Pretty print
    let output = serde_json::to_string_pretty(&json)
        .context("Failed to serialize to JSON")?;

    println!("{}", output);

    Ok(())
}

/// Convert a CBOR value to a JSON value
fn cbor_value_to_json(value: &ciborium::Value) -> serde_json::Value {
    use ciborium::Value;
    use serde_json::json;

    match value {
        Value::Integer(i) => {
            // Try to convert to i64 or u64
            let n: i128 = (*i).into();
            if n >= 0 && n <= u64::MAX as i128 {
                json!(n as u64)
            } else if n >= i64::MIN as i128 && n <= i64::MAX as i128 {
                json!(n as i64)
            } else {
                // Fall back to string for very large integers
                json!(n.to_string())
            }
        }
        Value::Bytes(b) => {
            // Try to decode as UTF-8 string, otherwise hex encode
            if let Ok(s) = std::str::from_utf8(b) {
                json!({ "__bytes_utf8": s })
            } else {
                json!({ "__bytes_hex": hex::encode(b) })
            }
        }
        Value::Float(f) => json!(f),
        Value::Text(s) => json!(s),
        Value::Bool(b) => json!(b),
        Value::Null => serde_json::Value::Null,
        Value::Tag(tag, inner) => {
            json!({
                "__tag": tag,
                "value": cbor_value_to_json(inner)
            })
        }
        Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(cbor_value_to_json).collect())
        }
        Value::Map(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                let key = match k {
                    Value::Text(s) => s.clone(),
                    Value::Integer(i) => {
                        let n: i128 = (*i).into();
                        n.to_string()
                    }
                    _ => format!("{:?}", k),
                };
                obj.insert(key, cbor_value_to_json(v));
            }
            serde_json::Value::Object(obj)
        }
        _ => json!(format!("{:?}", value)),
    }
}
