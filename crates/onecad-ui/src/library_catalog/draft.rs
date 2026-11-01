//! Pure "Try parameters" draft: per-field text, validation and the live designation.
//!
//! The validation mirrors `onecad-app`'s `validate_param_value` (the authority that
//! refuses a preview or a placement) so the page can explain a refusal before any
//! request is sent: role must be free, the value must be of the default's kind,
//! finite, a member of `domain` and at least `min`. It never decides anything the
//! service does not also check.
use onecad_app::dto::LibraryComponentDto;
use onecad_core::document::record::ComponentParamValue as Value;
use onecad_library::package::{ParameterRole, ParameterSpec};
use onecad_widgets::components::inputs::{ParsedValue, parse_value};
use std::collections::BTreeMap;

/// Converts a TOML scalar without naming the TOML type (this crate does not depend on it).
macro_rules! scalar {
    ($v:expr) => {{
        let v = $v;
        v.as_float()
            .map(|f| (Value::Number(f), false))
            .or_else(|| v.as_integer().map(|i| (Value::Number(i as f64), true)))
            .or_else(|| v.as_str().map(|s| (Value::Text(s.to_owned()), false)))
            .or_else(|| v.as_bool().map(|b| (Value::Bool(b), false)))
    }};
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Number,
    Text,
    Bool,
}

impl Kind {
    fn of(value: &Value) -> Self {
        match value {
            Value::Number(_) => Self::Number,
            Value::Text(_) => Self::Text,
            Value::Bool(_) => Self::Bool,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Text => "text",
            Self::Bool => "true / false",
        }
    }
}

/// One `domain` member; integer entries compare exactly like the service does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Allowed {
    pub value: Value,
    integral: bool,
}

impl Allowed {
    fn admits(&self, value: &Value) -> bool {
        match (&self.value, value) {
            (Value::Number(a), Value::Number(b)) if self.integral => a == b,
            (Value::Number(a), Value::Number(b)) => (a - b).abs() <= 1e-9 * a.abs().max(1.0),
            (Value::Text(a), Value::Text(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            _ => false,
        }
    }

    pub(crate) fn text(&self) -> String {
        value_text(&self.value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Field {
    pub key: String,
    pub role: ParameterRole,
    pub kind: Kind,
    pub default: Option<Value>,
    pub domain: Vec<Allowed>,
    pub min: Option<f64>,
    pub snap: Option<String>,
    pub from: Option<String>,
}

impl Field {
    pub(crate) fn from_spec(key: &str, spec: &ParameterSpec) -> Self {
        let default = spec.value.as_ref().and_then(|v| scalar!(v)).map(|s| s.0);
        let domain: Vec<Allowed> = spec
            .domain
            .iter()
            .flatten()
            .filter_map(|v| scalar!(v))
            .map(|(value, integral)| Allowed { value, integral })
            .collect();
        let kind = default
            .as_ref()
            .or_else(|| domain.first().map(|d| &d.value))
            .map_or(Kind::Number, Kind::of);
        Self {
            key: key.to_owned(),
            role: spec.role,
            kind,
            default,
            domain,
            min: spec.min,
            snap: spec.snap.clone(),
            from: spec.from.clone(),
        }
    }

    pub(crate) fn is_free(&self) -> bool {
        self.role == ParameterRole::Free
    }

    pub(crate) fn role_label(&self) -> &'static str {
        match self.role {
            ParameterRole::Free => "free",
            ParameterRole::Table => "table",
            ParameterRole::Computed => "computed",
        }
    }

    pub(crate) fn default_text(&self) -> String {
        self.default.as_ref().map(value_text).unwrap_or_default()
    }

    /// Parses `text` under this field's rules.
    pub(crate) fn parse(&self, text: &str) -> Result<Value, String> {
        let value = match self.kind {
            Kind::Number => match parse_value(text) {
                ParsedValue::Number(n) => Value::Number(n),
                ParsedValue::Expression(_) => {
                    return Err("Enter a number; expressions are not evaluated here".into());
                }
                ParsedValue::Invalid => return Err("Enter a finite number".into()),
            },
            Kind::Text => Value::Text(text.to_owned()),
            Kind::Bool => match text.trim() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => return Err("Choose true or false".into()),
            },
        };
        if !self.domain.is_empty() && !self.domain.iter().any(|d| d.admits(&value)) {
            return Err(format!("Not allowed; use one of {}", self.domain_list()));
        }
        // The service refuses any non-number once `min` is declared.
        match (&value, self.min) {
            (Value::Number(n), Some(min)) if *n < min => {
                Err(format!("Must be at least {}", format_number(min)))
            }
            (Value::Number(_), _) | (_, None) => Ok(value),
            (_, Some(min)) => Err(format!(
                "Must be a number of at least {}",
                format_number(min)
            )),
        }
    }

    fn domain_list(&self) -> String {
        const SHOWN: usize = 8;
        let mut list: Vec<String> = self.domain.iter().take(SHOWN).map(Allowed::text).collect();
        if self.domain.len() > SHOWN {
            list.push("…".into());
        }
        list.join(", ")
    }
}

/// Integral numbers without a trailing ".0" so field text and designation agree.
pub(crate) fn format_number(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Measured sizes at most two decimals, trailing zeros dropped.
pub(crate) fn format_dimension(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn value_text(value: &Value) -> String {
    match value {
        Value::Number(n) => format_number(*n),
        Value::Text(t) => t.clone(),
        Value::Bool(b) => b.to_string(),
    }
}

/// Field text for every free parameter, edited in place until previewed.
#[derive(Clone, Debug)]
pub(crate) struct ParamDraft {
    fields: Vec<Field>,
    designation: Option<String>,
    text: BTreeMap<String, String>,
}

impl ParamDraft {
    pub(crate) fn new(fields: Vec<Field>, designation: Option<String>) -> Self {
        let text = fields
            .iter()
            .filter(|f| f.is_free())
            .map(|f| (f.key.clone(), f.default_text()))
            .collect();
        Self {
            fields,
            designation,
            text,
        }
    }

    pub(crate) fn from_component(component: &LibraryComponentDto) -> Self {
        let template = component.designation.clone();
        let mut fields: Vec<Field> = component
            .parameters
            .iter()
            .map(|(key, spec)| Field::from_spec(key, spec))
            .collect();
        // Parameters named by the designation lead, in the order a reader meets them.
        let rank = |key: &str| {
            template
                .as_deref()
                .and_then(|t| t.find(&format!("{{{key}}}")))
                .unwrap_or(usize::MAX)
        };
        fields.sort_by_key(|f| rank(&f.key));
        Self::new(fields, template)
    }

    pub(crate) fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub(crate) fn text(&self, key: &str) -> &str {
        self.text.get(key).map_or("", String::as_str)
    }

    pub(crate) fn text_mut(&mut self, key: &str) -> Option<&mut String> {
        self.text.get_mut(key)
    }

    pub(crate) fn set_text(&mut self, key: &str, text: String) {
        if let Some(slot) = self.text.get_mut(key) {
            *slot = text;
        }
    }

    /// Sets each free field named in `params` to that value's text.
    pub(crate) fn load(&mut self, params: &BTreeMap<String, Value>) {
        for (key, value) in params {
            self.set_text(key, value_text(value));
        }
    }

    pub(crate) fn reset(&mut self) {
        for field in self.fields.iter().filter(|f| f.is_free()) {
            self.text.insert(field.key.clone(), field.default_text());
        }
    }

    pub(crate) fn is_default(&self) -> bool {
        self.fields
            .iter()
            .filter(|f| f.is_free())
            .all(|f| self.text(&f.key) == f.default_text())
    }

    /// Every free parameter's value, or the per-field refusals.
    pub(crate) fn resolve(&self) -> Result<BTreeMap<String, Value>, BTreeMap<String, String>> {
        let mut values = BTreeMap::new();
        let mut errors = BTreeMap::new();
        for field in self.fields.iter().filter(|f| f.is_free()) {
            match field.parse(self.text(&field.key)) {
                Ok(value) => {
                    values.insert(field.key.clone(), value);
                }
                Err(error) => {
                    errors.insert(field.key.clone(), error);
                }
            }
        }
        if errors.is_empty() {
            Ok(values)
        } else {
            Err(errors)
        }
    }

    pub(crate) fn errors(&self) -> BTreeMap<String, String> {
        self.resolve().err().unwrap_or_default()
    }

    /// The designation template with each `{key}` replaced by its resolved value; a
    /// key whose field is refused (or has no value) stays as `{key}` rather than
    /// showing text the request would not send.
    pub(crate) fn designation(&self) -> Option<String> {
        let template = self.designation.as_deref()?;
        let lookup = |key: &str| -> Option<String> {
            let field = self.fields.iter().find(|f| f.key == key)?;
            if field.is_free() {
                field.parse(self.text(key)).ok().map(|v| value_text(&v))
            } else {
                field.default.as_ref().map(value_text)
            }
        };
        Some(substitute(template, lookup))
    }
}

fn substitute(template: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        let Some(close) = tail.find('}') else {
            out.push_str(tail);
            return out;
        };
        let key = &tail[1..close];
        match lookup(key) {
            Some(text) => out.push_str(&text),
            None => out.push_str(&tail[..=close]),
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}
