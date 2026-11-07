//! Options model.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Default,
    Create,
    Replace,
    Delete,
    List,
    Extract,
    Thin,
    Deterministic,
}

#[derive(Debug, Clone)]
pub struct ArOptions {
    pub mode: Mode,
    pub verbose: bool,
}

impl Default for ArOptions {
    fn default() -> Self { Self { mode: Mode::Default, verbose: false } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opts() { assert_eq!(ArOptions::default().mode, Mode::Default); }
}
