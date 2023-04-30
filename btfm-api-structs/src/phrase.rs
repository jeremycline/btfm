use serde::{Deserialize, Serialize};

/// A phrase used to trigger one or more clips
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Phrase {
    pub uuid: String,
    pub phrase: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreatePhrase {
    /// The phrase.
    pub phrase: String,
    /// The clip to associate the phrase to.
    pub clip: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Phrases {
    pub items: u64,
    pub phrases: Vec<Phrase>,
}
