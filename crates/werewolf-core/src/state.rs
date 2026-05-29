use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WolfMode {
    Human,
    Wolf,
    Silver,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub mode: WolfMode,
    pub pelt_ready: bool,
    pub packmates: usize,
    pub active_fangs: usize,
    pub silver: String,
    pub hide: String,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            mode: WolfMode::Human,
            pelt_ready: false,
            packmates: 0,
            active_fangs: 0,
            silver: "armed".to_string(),
            hide: "light".to_string(),
        }
    }
}
