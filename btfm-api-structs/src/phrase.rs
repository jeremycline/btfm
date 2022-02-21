use serde::{Deserialize, Serialize};

/// A phrase used to trigger one or more clips
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Phrase {
    pub ulid: ulid::Ulid,
    pub phrase: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreatePhrase {
    /// The phrase.
    pub phrase: String,
    /// The clip to associate the phrase to.
    pub clip: ulid::Ulid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Phrases {
    pub items: u64,
    pub phrases: Vec<Phrase>,
}
