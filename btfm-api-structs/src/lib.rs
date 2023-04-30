/// Defines public-facing structures used in the web API
use serde::{Deserialize, Serialize};

mod clip;
mod phrase;

pub use clip::{Clip, ClipUpdated, ClipUpload, Clips};
pub use phrase::{CreatePhrase, Phrase, Phrases};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Status {
    pub db_connections: u32,
}
