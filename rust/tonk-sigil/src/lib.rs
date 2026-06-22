mod sigil;

pub use sigil::Sigil;

#[cfg(feature = "web")]
mod web;

#[cfg(feature = "web")]
pub use web::set_default_sprite_href;
