use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Discuss,
    Building,
    Tour,
    Applied,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EditorContext {
    pub file: Option<String>,
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub column: usize,
    #[serde(default)]
    pub selection: String,
    #[serde(default)]
    pub selection_start_line: Option<usize>,
    #[serde(default)]
    pub selection_end_line: Option<usize>,
    #[serde(default)]
    pub nearby: String,
    #[serde(default)]
    pub dirty: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TourStop {
    pub title: String,
    pub body: String,
    pub file: String,
    pub line: usize,
    /// Inclusive, 1-based end of the reading range.
    pub end_line: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TourDraft {
    pub title: String,
    pub overview: String,
    pub stops: Vec<TourStop>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "proposal", rename_all = "snake_case")]
pub enum TourSource {
    Repository,
    Proposal(usize),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tour {
    pub id: usize,
    pub source: TourSource,
    pub current_stop: usize,
    #[serde(flatten)]
    pub content: TourDraft,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub id: usize,
    pub file: String,
    pub line: usize,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub id: usize,
    pub base: String,
    pub tree: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub stage: Stage,
    pub busy: bool,
    pub real: String,
    pub shadow: String,
    pub proposal: Option<usize>,
    pub proposals: Vec<usize>,
    pub tour: Option<Tour>,
    pub changes: Vec<Change>,
    pub change_index: usize,
    pub editor: EditorContext,
    pub navigation: Option<Location>,
    pub generation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Status,
    Input { text: String },
    Message { text: String },
    Begin { text: String },
    Revise { feedback: String },
    Switch { proposal: usize },
    TourNext,
    TourPrevious,
    TourJump { index: usize },
    TourClose,
    Apply,
    NextChange,
    PreviousChange,
    Revert { change: usize },
    Diff,
    Peek,
    Context { context: EditorContext },
    EditorConnection { connected: bool },
    Cancel,
    Shutdown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    #[serde(flatten)]
    pub action: Action,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Comparison {
    pub file: String,
    pub current_file: String,
    pub old: Option<String>,
    pub current: Option<String>,
    pub old_line: usize,
    pub current_line: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Payload {
    pub text: Option<String>,
    pub comparison: Option<Comparison>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    pub view: Option<View>,
    pub error: Option<String>,
    #[serde(flatten)]
    pub reply: Payload,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    History { lines: Vec<String> },
    State { view: Box<View> },
    Message { text: String },
    Activity { text: String },
    Error { text: String },
}
