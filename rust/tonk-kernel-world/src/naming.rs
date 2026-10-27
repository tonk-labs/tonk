//! The name of the layer a location of Dialog DB opens, and back: the guest
//! kernel names each layer it opens for a location, and the host keeps the
//! layer where Dialog DB keeps that location, so a store Dialog DB kept
//! before opens as the layer it was.

use dialog_effects::storage::{Directory, Location};

/// The name of the layer `location` opens. A space, which Tonk opens in the
/// current directory under its subject's DID, opens the layer of that
/// subject: the one its host boots it over, which holds its boot cell and its
/// kernel. Anything else opens a layer named by a DID of the method `local`,
/// with each character a DID does not allow escaped.
pub fn layer_name(location: &Location) -> String {
    if location.directory == Directory::Current
        && location.name.starts_with("did:")
        && !location.name.starts_with("did:local:")
    {
        return location.name.clone();
    }
    let directory = match &location.directory {
        Directory::Profile => "profile".to_owned(),
        Directory::Current => "current".to_owned(),
        Directory::Temp => "temp".to_owned(),
        Directory::At(path) => format!("at:{path}"),
    };
    let escape = |text: &str| {
        text.bytes()
            .map(|byte| match byte {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' => {
                    (byte as char).to_string()
                }
                other => format!("%{other:02X}"),
            })
            .collect::<String>()
    };
    format!(
        "did:local:{}:{}",
        escape(&directory),
        escape(&location.name)
    )
}

/// The location of Dialog DB the layer `name` is kept at: the one
/// [`layer_name`] names it for. Any name but a `did:local:` one is a space's
/// subject, in the current directory.
pub fn location(name: &str) -> Location {
    let Some(rest) = name.strip_prefix("did:local:") else {
        return Location::new(Directory::Current, name);
    };
    let (directory, named) = rest.split_once(':').unwrap_or(("current", rest));
    let directory = match unescape(directory).as_str() {
        "profile" => Directory::Profile,
        "temp" => Directory::Temp,
        other => match other.strip_prefix("at:") {
            Some(path) => Directory::At(path.to_owned()),
            None => Directory::Current,
        },
    };
    Location::new(directory, unescape(named))
}

/// `text`, with each `%XX` escape [`layer_name`] wrote put back.
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && let Some(byte) = text
                .get(at + 1..at + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_names_a_space_by_its_subject() {
        let space = Location::new(Directory::Current, "did:key:z6MkSpace");
        assert_eq!(layer_name(&space), "did:key:z6MkSpace");
        assert_eq!(location("did:key:z6MkSpace"), space);
    }

    #[test]
    fn it_finds_the_location_a_layer_was_named_for() {
        for named in [
            Location::new(Directory::Profile, "tonk"),
            Location::new(Directory::Profile, "dialog.credential"),
            Location::new(Directory::Temp, "scratch pad"),
            Location::new(Directory::At("/home/me".to_owned()), "a:b"),
        ] {
            assert_eq!(location(&layer_name(&named)), named, "{named:?}");
        }
    }
}
