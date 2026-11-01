//! `ParamDraft`: validation, resolved values and the live designation.
use super::{
    draft::{Field, ParamDraft},
    testing::{component, iso_draft, iso_specs, spec},
};
use onecad_core::document::record::ComponentParamValue as Value;
use serde_json::json;

#[test]
fn defaults_resolve_and_name_the_designation() {
    let draft = iso_draft();
    let keys: Vec<&str> = draft.fields().iter().map(|f| f.key.as_str()).collect();
    // Designation order first, then the rest alphabetically.
    assert_eq!(keys, ["thread", "length", "pitch", "thread_detail"]);
    let params = draft.resolve().expect("defaults are valid");
    assert_eq!(params["thread"], Value::Text("M6".into()));
    assert_eq!(params["length"], Value::Number(20.0));
    assert_eq!(params["thread_detail"], Value::Text("cosmetic".into()));
    // Table parameters are the component's, never part of a request.
    assert!(!params.contains_key("pitch"));
    assert_eq!(draft.designation().as_deref(), Some("ISO 4762 M6x20"));
    assert!(draft.is_default());
}

#[test]
fn value_outside_the_domain_is_refused_and_left_unresolved() {
    let mut draft = iso_draft();
    draft.set_text("thread", "M7".into());
    let errors = draft.resolve().expect_err("M7 is not a size");
    assert!(errors["thread"].starts_with("Not allowed; use one of M2, M2.5"));
    assert_eq!(errors.len(), 1);
    // The refused field is shown as the placeholder, not as text a request would not send.
    assert_eq!(draft.designation().as_deref(), Some("ISO 4762 {thread}x20"));
}

#[test]
fn value_below_min_is_refused() {
    let mut draft = iso_draft();
    draft.set_text("length", "1.5".into());
    assert_eq!(draft.errors()["length"], "Must be at least 2");
    draft.set_text("length", "2".into());
    assert!(draft.resolve().is_ok());
}

#[test]
fn wrong_kind_or_non_finite_text_is_refused() {
    let mut draft = iso_draft();
    for bad in ["abc", "", "inf", "NaN", "=1+1"] {
        draft.set_text("length", bad.into());
        assert!(
            draft.errors().contains_key("length"),
            "{bad:?} must be refused"
        );
    }
    assert_eq!(
        draft.errors()["length"],
        "Enter a number; expressions are not evaluated here"
    );
    draft.set_text("length", "abc".into());
    assert_eq!(draft.errors()["length"], "Enter a finite number");
    let bool_field = Field::from_spec("fillets", &spec(json!({"role": "free", "value": true})));
    assert!(bool_field.parse("yes").is_err());
    assert_eq!(bool_field.parse("false"), Ok(Value::Bool(false)));
}

#[test]
fn field_text_resolved_values_and_designation_agree() {
    let mut draft = iso_draft();
    for (thread, length, shown) in [
        ("M8", "25.0", "ISO 4762 M8x25"),
        ("M10", " 12.50 ", "ISO 4762 M10x12.5"),
        ("M3", "1e1", "ISO 4762 M3x10"),
        ("M12", "30", "ISO 4762 M12x30"),
    ] {
        draft.set_text("thread", thread.into());
        draft.set_text("length", length.into());
        let params = draft.resolve().expect("valid");
        let Value::Number(resolved) = params["length"] else {
            panic!("length is numeric");
        };
        assert_eq!(params["thread"], Value::Text(thread.into()));
        assert_eq!(draft.designation().as_deref(), Some(shown));
        // The designation's length is exactly the resolved number, formatted.
        let tail = shown.rsplit('x').next().expect("length part");
        assert_eq!(tail.parse::<f64>().expect("number"), resolved);
    }
}

#[test]
fn integer_domains_compare_exactly_and_float_domains_with_tolerance() {
    let sizes = Field::from_spec(
        "sizes",
        &spec(json!({"role": "free", "value": 10, "domain": [10, 12.5]})),
    );
    assert_eq!(sizes.parse("10"), Ok(Value::Number(10.0)));
    assert!(sizes.parse("11").is_err());
    assert!(sizes.parse("12.5000000001").is_ok());
    assert!(sizes.parse("12.51").is_err());
}

#[test]
fn locked_parameters_keep_their_value_and_reset_restores_defaults() {
    let mut draft = iso_draft();
    let pitch = draft
        .fields()
        .iter()
        .find(|f| f.key == "pitch")
        .expect("pitch");
    assert!(!pitch.is_free());
    assert_eq!(pitch.from.as_deref(), Some("pitch_mm"));
    assert!(
        draft.text_mut("pitch").is_none(),
        "locked fields have no editable text"
    );
    draft.set_text("length", "44".into());
    assert!(!draft.is_default());
    draft.reset();
    assert!(draft.is_default());
    assert_eq!(draft.text("length"), "20");
}

#[test]
fn designation_uses_a_locked_default_and_survives_unknown_keys() {
    let mut specs = iso_specs();
    specs.insert(
        "grade".into(),
        spec(json!({"role": "computed", "value": "A2"})),
    );
    let mut c = component("iso", "1.0.0", specs);
    c.designation = Some("{grade} {thread}x{length} {nope} {open".into());
    let draft = ParamDraft::from_component(&c);
    assert_eq!(
        draft.designation().as_deref(),
        Some("A2 M6x20 {nope} {open")
    );
}

#[test]
fn min_refuses_any_non_number_like_the_service() {
    let text = Field::from_spec(
        "label",
        &spec(json!({"role": "free", "value": "a", "min": 2})),
    );
    assert_eq!(
        text.parse("x"),
        Err("Must be a number of at least 2".to_owned())
    );
    let flag = Field::from_spec(
        "flag",
        &spec(json!({"role": "free", "value": true, "min": 1})),
    );
    assert!(flag.parse("true").is_err());
}
