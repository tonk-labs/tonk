//! The origin a request came from, as the router's handlers see it.

use url::Url;

/// Same-origin scheme and authority captured from the request URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestOrigin(Url);

impl RequestOrigin {
    /// Capture only the URL origin, discarding path, query, and fragment.
    pub fn parse(request_url: &str) -> Result<Self, url::ParseError> {
        let origin = Url::parse(request_url)?.origin().ascii_serialization();
        Ok(Self(Url::parse(&format!("{origin}/"))?))
    }

    /// Origin URL with a root path and no query or fragment.
    pub fn url(&self) -> &Url {
        &self.0
    }
}
