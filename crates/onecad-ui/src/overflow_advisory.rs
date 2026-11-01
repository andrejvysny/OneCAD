//! Non-blocking advisory for a Fillet/Chamfer whose blend was trimmed by the
//! overflow opt-in. A warning explains geometry; it never gates Create/Apply.
use crate::app::App;
use onecad_core::regen::{Diagnostic, Severity};
use onecad_interaction::ModelSession;

/// Text for the first `*_OVERFLOW_TRIMMED` warning, chosen by code and
/// `evidence.overflowKind`. Fillets and chamfers trim differently, so a kind
/// that does not belong to the code yields no text rather than a wrong one.
pub(crate) fn text(diagnostics: &[Diagnostic]) -> Option<String> {
    diagnostics.iter().find_map(|d| {
        if d.severity != Severity::Warning {
            return None;
        }
        let kind = d.evidence.as_ref()?.get("overflowKind")?.as_str()?;
        let how = match (d.code.as_str(), kind) {
            ("FILLET_OVERFLOW_TRIMMED", "cliff") => "cliff",
            ("CHAMFER_OVERFLOW_TRIMMED", "extendedPlane") => "extended plane",
            _ => return None,
        };
        Some(format!(
            "Blend overflows the adjacent face; trimmed like Shapr3D Auto ({how})"
        ))
    })
}

/// Advisory for the feature being edited, from its committed step diagnostics.
pub(crate) fn for_tool(app: &App, tool: &ModelSession) -> Option<String> {
    if !tool.editing_existing {
        return None;
    }
    let id = tool.record.record_id.to_string();
    let feature = app
        .doc
        .display
        .as_ref()?
        .projection
        .features
        .iter()
        .find(|f| f.id == id)?;
    text(&feature.diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn diag(severity: Severity, code: &str, kind: &str) -> Diagnostic {
        Diagnostic {
            severity,
            code: code.into(),
            message: "m".into(),
            stage: Some("publication".into()),
            reason_code: None,
            evidence: Some(json!({ "overflowKind": kind })),
        }
    }

    #[test]
    fn fillet_and_chamfer_texts_follow_code_and_kind() {
        let fillet = [diag(Severity::Warning, "FILLET_OVERFLOW_TRIMMED", "cliff")];
        assert_eq!(
            text(&fillet).as_deref(),
            Some("Blend overflows the adjacent face; trimmed like Shapr3D Auto (cliff)")
        );
        let chamfer = [diag(
            Severity::Warning,
            "CHAMFER_OVERFLOW_TRIMMED",
            "extendedPlane",
        )];
        assert!(text(&chamfer).unwrap().ends_with("(extended plane)"));
    }

    #[test]
    fn unrelated_or_non_warning_diagnostics_give_no_text() {
        assert!(text(&[diag(Severity::Error, "FILLET_OVERFLOW_TRIMMED", "cliff")]).is_none());
        assert!(
            text(&[diag(
                Severity::Warning,
                "FILLET_BLEND_APPROXIMATED",
                "cliff"
            )])
            .is_none()
        );
        assert!(
            text(&[diag(
                Severity::Warning,
                "FILLET_OVERFLOW_TRIMMED",
                "extendedPlane"
            )])
            .is_none()
        );
    }

    #[test]
    fn advisory_text_does_not_touch_preview_failure() {
        // The advisory is a separate row; the preview status stays Ready so
        // `ModelSession::ready` (and Create/Apply) are unaffected by it.
        let fillet = [diag(Severity::Warning, "FILLET_OVERFLOW_TRIMMED", "cliff")];
        assert!(text(&fillet).is_some());
        let feedback = crate::feature_card::preview_feedback_for_test();
        assert!(feedback.is_none());
    }
}
