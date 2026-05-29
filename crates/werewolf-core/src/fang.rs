use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FangState {
    Opening,
    Active,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FangRecord {
    pub id: String,
    pub peer: String,
    pub local: String,
    pub remote: String,
    pub state: FangState,
}
