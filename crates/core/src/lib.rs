#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VitisTask {
    pub id: usize,
    pub instruction: String,
    pub indent_level: usize,
    pub depends_on: Vec<usize>,
}

impl VitisTask {
    pub fn new(
        id: usize,
        instruction: impl Into<String>,
        indent_level: usize,
        depends_on: Vec<usize>,
    ) -> Self {
        Self {
            id,
            instruction: instruction.into(),
            indent_level,
            depends_on,
        }
    }
}
