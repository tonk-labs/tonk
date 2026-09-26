//! Typed requests that temporarily replace a FABB from the trusted page.
//!
//! A sealed guest may describe purpose and geometry. It cannot send markup or
//! name a renderer. The trusted page chooses which purposes it supports and
//! keeps all privileged effects in its own document.

use serde::{Deserialize, Serialize};

/// Version of the contained-task wire contract.
pub const VERSION: u8 = 1;

/// A request carried through the portal message port.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// Wire format version.
    pub version: u8,
    /// Identity allocated by the originating FABB.
    pub request_id: String,
    /// The trusted-page capability being requested.
    pub purpose: Purpose,
    /// Whether this opens, moves, hides, shows or closes the standing task.
    pub action: Action,
    /// Narrow account-flow context retained across the trusted presentation.
    /// This is data, never guest-provided markup or a renderer name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountContext>,
    /// Validated viewport geometry for the originating FABB.
    #[serde(default)]
    pub presentation: Option<Presentation>,
}

impl Request {
    /// Parse and validate a guest request.
    pub fn parse(payload: &str) -> Result<Self, ParseError> {
        let request: Self = serde_json::from_str(payload).map_err(|_| ParseError::Malformed)?;
        request.validate()?;
        Ok(request)
    }

    /// Validate fields that serde's types alone cannot constrain.
    pub fn validate(&self) -> Result<(), ParseError> {
        if self.version != VERSION {
            return Err(ParseError::Version);
        }
        if self.request_id.is_empty() || self.request_id.len() > 128 {
            return Err(ParseError::RequestId);
        }
        if let Some(presentation) = &self.presentation {
            presentation.validate()?;
        }
        match self.action {
            Action::Open | Action::Reseat if self.presentation.is_none() => {
                Err(ParseError::Presentation)
            }
            _ => Ok(()),
        }?;
        if let Some(account) = &self.account
            && (self.purpose != Purpose::Account
                || account.reason.is_empty()
                || account.reason.len() > 128
                || account.space.len() > 1024)
        {
            return Err(ParseError::AccountContext);
        }
        Ok(())
    }

    /// Translate guest viewport coordinates through one live iframe.
    pub fn translated(mut self, x: f64, y: f64) -> Result<Self, ParseError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(ParseError::Presentation);
        }
        if let Some(presentation) = self.presentation.as_mut() {
            presentation.anchor.left += x;
            presentation.anchor.right += x;
            presentation.anchor.top += y;
            presentation.anchor.bottom += y;
            presentation.validate()?;
        }
        Ok(self)
    }

    /// Serialize a validated request for a nested portal relay.
    pub fn to_json(&self) -> Result<String, ParseError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|_| ParseError::Malformed)
    }
}

/// Safe resume data for a trusted account ceremony.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountContext {
    /// Why account capability is needed (for example, a blocked share).
    pub reason: String,
    /// The space DID whose action should resume after completion.
    #[serde(default)]
    pub space: String,
}

/// Capabilities the trusted page may choose to present.
/// Horizontal edge retained while the trusted surface changes width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Purpose {
    /// An account or sign-in ceremony. Privileged credentials stay in the page.
    Account,
    /// A side-effect-free transport probe used by the browser contract test.
    Probe,
}

/// Lifecycle operation for a standing request.
/// Vertical edge retained while the trusted surface changes height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Start a task and allocate its one result path.
    Open,
    /// Move the standing task to the latest translated anchor.
    Reseat,
    /// Hide the standing task without destroying its state.
    Suspend,
    /// Reveal a suspended task.
    Show,
    /// Tear down the standing task.
    Dismiss,
}

/// Presentation metadata accepted by the trusted host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presentation {
    /// The FABB rectangle in the current host viewport.
    pub anchor: Anchor,
    /// Which horizontal edge must stay fixed as the task changes width.
    pub horizontal: Horizontal,
    /// Which vertical edge must stay fixed as the task changes height.
    pub vertical: Vertical,
    /// Whether Escape may cancel the task.
    pub dismissal: Dismissal,
}

impl Presentation {
    fn validate(&self) -> Result<(), ParseError> {
        self.anchor.validate()
    }
}

/// The originating FABB's viewport rectangle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    /// Left viewport edge in CSS pixels.
    pub left: f64,
    /// Top viewport edge in CSS pixels.
    pub top: f64,
    /// Right viewport edge in CSS pixels.
    pub right: f64,
    /// Bottom viewport edge in CSS pixels.
    pub bottom: f64,
    /// Rectangle width in CSS pixels.
    pub width: f64,
    /// Rectangle height in CSS pixels.
    pub height: f64,
}

impl Anchor {
    fn validate(&self) -> Result<(), ParseError> {
        let values = [
            self.left,
            self.top,
            self.right,
            self.bottom,
            self.width,
            self.height,
        ];
        if values.iter().any(|value| !value.is_finite())
            || self.width <= 0.0
            || self.height <= 0.0
            || self.right < self.left
            || self.bottom < self.top
            || (self.right - self.left - self.width).abs() > 1.0
            || (self.bottom - self.top - self.height).abs() > 1.0
        {
            return Err(ParseError::Presentation);
        }
        Ok(())
    }
}

/// Horizontal edge retained while the trusted surface changes width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Horizontal {
    /// Preserve the left edge.
    Left,
    /// Preserve the right edge.
    Right,
}

/// Vertical edge retained while the trusted surface changes height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Vertical {
    /// Preserve the top edge.
    Top,
    /// Preserve the bottom edge.
    Bottom,
}

/// Whether the task may be dismissed without an explicit action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dismissal {
    /// Escape may cancel the task.
    Optional,
    /// Only an explicit acknowledgement or completion may close the task.
    Required,
}

/// Why a guest task request was rejected before reaching a presenter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The JSON shape or field types are invalid.
    Malformed,
    /// The wire version is unsupported.
    Version,
    /// The request identity is absent or unreasonably large.
    RequestId,
    /// Required geometry is absent, inconsistent or non-finite.
    Presentation,
    /// Account context was empty, oversized or attached to another purpose.
    AccountContext,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            version: VERSION,
            request_id: "task-1".into(),
            purpose: Purpose::Account,
            action: Action::Open,
            account: Some(AccountContext {
                reason: "needs-account".into(),
                space: "did:key:space".into(),
            }),
            presentation: Some(Presentation {
                anchor: Anchor {
                    left: 10.0,
                    top: 20.0,
                    right: 110.0,
                    bottom: 68.0,
                    width: 100.0,
                    height: 48.0,
                },
                horizontal: Horizontal::Right,
                vertical: Vertical::Bottom,
                dismissal: Dismissal::Optional,
            }),
        }
    }

    #[test]
    fn it_parses_only_versioned_typed_requests_with_consistent_geometry() {
        let encoded = request().to_json().expect("encode request");
        assert_eq!(Request::parse(&encoded), Ok(request()));

        let mut malformed = request();
        malformed.presentation.as_mut().unwrap().anchor.width = 90.0;
        assert_eq!(malformed.validate(), Err(ParseError::Presentation));
        malformed.presentation = None;
        assert_eq!(malformed.validate(), Err(ParseError::Presentation));

        let mut wrong_purpose = request();
        wrong_purpose.purpose = Purpose::Probe;
        assert_eq!(wrong_purpose.validate(), Err(ParseError::AccountContext));
    }

    #[test]
    fn it_translates_each_edge_through_one_frame_without_changing_size() {
        let translated = request().translated(30.0, 40.0).expect("translate");
        let anchor = translated.presentation.unwrap().anchor;
        assert_eq!(anchor.left, 40.0);
        assert_eq!(anchor.right, 140.0);
        assert_eq!(anchor.top, 60.0);
        assert_eq!(anchor.bottom, 108.0);
        assert_eq!(anchor.width, 100.0);
        assert_eq!(anchor.height, 48.0);
    }
}
