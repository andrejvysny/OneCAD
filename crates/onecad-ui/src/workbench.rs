//! Active workbench (title-bar workspace switcher selection).
//! UI-only presentation state: not document state, not persisted.
//! Tool filtering per workbench is follow-up (ONECAD-397/398); until then every
//! workbench shows the same tools and chrome.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Workbench {
    #[default]
    Design,
    Assembly,
}

impl Workbench {
    pub const ALL: [Workbench; 2] = [Workbench::Design, Workbench::Assembly];

    pub fn label(self) -> &'static str {
        match self {
            Workbench::Design => "Design",
            Workbench::Assembly => "Assembly",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_design_and_labels_match() {
        assert_eq!(Workbench::default(), Workbench::Design);
        assert_eq!(Workbench::Design.label(), "Design");
        assert_eq!(Workbench::Assembly.label(), "Assembly");
        assert_eq!(Workbench::ALL.len(), 2);
    }
}
