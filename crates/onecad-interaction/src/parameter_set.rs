//! Declared parameter fields of one model session. The base operation holds
//! choices and placement; active fields override its slots: Valid fields with
//! their evaluated scalar, Authored fields with the slot captured at begin, so
//! untouched fields keep their authored scalar (and expression) verbatim.
use crate::NumericParameter;
use onecad_core::document::{
    record::{Operation, OperationRecord},
    variables::Scalar,
};

/// One editable parameter slot. Pattern counts join as their own variant in
/// the lane that authors them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterKey {
    Scalar(NumericParameter),
}

impl From<NumericParameter> for ParameterKey {
    fn from(parameter: NumericParameter) -> Self {
        Self::Scalar(parameter)
    }
}

impl ParameterKey {
    pub fn parameter(self) -> NumericParameter {
        match self {
            Self::Scalar(parameter) => parameter,
        }
    }
    pub fn label(self) -> &'static str {
        self.parameter().label()
    }
    pub fn site(self) -> &'static str {
        self.parameter().site()
    }
    pub fn salt(self) -> &'static str {
        self.parameter().salt()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldStatus {
    /// Text equals the authored original on an existing edit: the slot captured at
    /// begin is used verbatim, whatever placement later wrote into the base.
    Authored,
    Pending,
    Valid(Scalar),
    /// Expression error; it stays until the text changes.
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterField {
    pub key: ParameterKey,
    /// Authored text at session begin; never rewritten during the session.
    pub original: String,
    pub text: String,
    pub status: FieldStatus,
    pub active: bool,
    /// Slot at session begin; an Authored field commits it verbatim.
    pub(crate) authored: Scalar,
    /// Fresh evaluation of the untouched original; the stored value may be a stale cache.
    pub(crate) authored_value: Option<f64>,
}

impl ParameterField {
    /// Authored slot with its freshest known value, for display and placement.
    pub(crate) fn authored_shown(&self) -> Scalar {
        Scalar {
            value: self.authored_value.unwrap_or(self.authored.value),
            expr: self.authored.expr.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Readiness {
    Ready,
    Pending,
    Invalid {
        key: Option<ParameterKey>,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterSet {
    pub(crate) fields: Vec<ParameterField>,
    pub(crate) focus: Option<ParameterKey>,
    /// Field bound to the session's current request sequence.
    pub(crate) evaluating: Option<ParameterKey>,
    /// Candidate/operation error, recomputed on every preview request.
    pub(crate) op_error: Option<(Option<ParameterKey>, String)>,
    pub(crate) last_edited: Option<ParameterKey>,
}

/// Reducer-owned state at drag begin; a drag cancel restores it exactly.
#[derive(Clone, Debug)]
pub(crate) struct DragOrigin {
    pub(crate) record: OperationRecord,
    pub(crate) fields: Vec<ParameterField>,
    pub(crate) focus: Option<ParameterKey>,
}

/// Keys of every active, unambiguous scalar slot of `op`, in registry order.
pub fn declared_keys(op: &Operation) -> Vec<ParameterKey> {
    crate::numeric_parameters(op)
        .into_iter()
        .map(|descriptor| descriptor.parameter.into())
        .collect()
}

impl ParameterSet {
    pub(crate) fn declare(
        op: &Operation,
        keys: &[ParameterKey],
        focus: Option<ParameterKey>,
    ) -> Result<Self, String> {
        let mut fields: Vec<ParameterField> = Vec::with_capacity(keys.len());
        for key in keys {
            if fields.iter().any(|field| field.key == *key) {
                return Err(format!("{} is declared twice", key.label()));
            }
            let scalar = key.parameter().registered_scalar(op)?;
            let original = scalar
                .expr
                .clone()
                .unwrap_or_else(|| scalar.value.to_string());
            fields.push(ParameterField {
                key: *key,
                text: original.clone(),
                original,
                status: FieldStatus::Authored,
                active: key.parameter().is_active(op),
                authored: scalar,
                authored_value: None,
            });
        }
        if let Some(focus) = focus {
            if !fields
                .iter()
                .any(|field| field.key == focus && field.active)
            {
                return Err(focus.parameter().unsupported());
            }
        }
        Ok(Self {
            fields,
            focus,
            evaluating: None,
            op_error: None,
            last_edited: None,
        })
    }

    pub fn fields(&self) -> &[ParameterField] {
        &self.fields
    }
    pub fn active(&self) -> impl Iterator<Item = &ParameterField> {
        self.fields.iter().filter(|field| field.active)
    }
    pub fn field(&self, key: ParameterKey) -> Option<&ParameterField> {
        self.fields.iter().find(|field| field.key == key)
    }
    pub(crate) fn field_mut(&mut self, key: ParameterKey) -> Option<&mut ParameterField> {
        self.fields.iter_mut().find(|field| field.key == key)
    }
    pub fn focus(&self) -> Option<ParameterKey> {
        self.focus
    }

    /// Error shown for `key`: its expression error, else an attributed candidate error.
    pub fn error(&self, key: ParameterKey) -> Option<&str> {
        match (&self.field(key)?.status, &self.op_error) {
            (FieldStatus::Invalid(message), _) => Some(message),
            (_, Some((Some(owner), message))) if *owner == key => Some(message),
            _ => None,
        }
    }

    /// Over active fields only; expression errors win over pending evaluation.
    pub fn readiness(&self) -> Readiness {
        let invalid = self.active().find_map(|field| match &field.status {
            FieldStatus::Invalid(message) => Some((field.key, message)),
            _ => None,
        });
        if let Some((key, message)) = invalid {
            return Readiness::Invalid {
                key: Some(key),
                message: message.clone(),
            };
        }
        if let Some((key, message)) = &self.op_error {
            return Readiness::Invalid {
                key: *key,
                message: message.clone(),
            };
        }
        if self.first_pending().is_some() {
            return Readiness::Pending;
        }
        Readiness::Ready
    }

    pub(crate) fn first_pending(&self) -> Option<ParameterKey> {
        self.active()
            .find(|field| field.status == FieldStatus::Pending)
            .map(|field| field.key)
    }

    pub(crate) fn is_pending(&self, key: ParameterKey) -> bool {
        self.field(key)
            .is_some_and(|field| field.active && field.status == FieldStatus::Pending)
    }

    /// Evaluated value of `key`; None while its text is pending or invalid.
    pub fn value(&self, key: ParameterKey) -> Option<f64> {
        let field = self.field(key).filter(|field| field.active)?;
        if self.error(key).is_some() {
            return None;
        }
        match &field.status {
            FieldStatus::Valid(scalar) => Some(scalar.value),
            FieldStatus::Authored => Some(field.authored_shown().value),
            FieldStatus::Pending | FieldStatus::Invalid(_) => None,
        }
    }

    /// Base with every active override written, unvalidated, Authored fields at
    /// their freshest value: what placement handles show.
    pub(crate) fn overlay(&self, base: &Operation) -> Operation {
        let mut op = base.clone();
        for field in self.active() {
            let scalar = match &field.status {
                FieldStatus::Valid(scalar) => scalar.clone(),
                FieldStatus::Authored => field.authored_shown(),
                FieldStatus::Pending | FieldStatus::Invalid(_) => continue,
            };
            let _ = field.key.parameter().write(&mut op, scalar);
        }
        op
    }

    /// The operation both preview and commit use: same function, same inputs.
    pub(crate) fn candidate(
        &self,
        base: &Operation,
    ) -> Result<Operation, (Option<ParameterKey>, String)> {
        let mut op = base.clone();
        // Untouched slots are written back verbatim, never re-validated: a broken
        // authored binding must not block the other fields.
        for field in self.active() {
            if field.status == FieldStatus::Authored {
                let _ = field.key.parameter().write(&mut op, field.authored.clone());
            }
        }
        for (key, scalar) in self.overrides() {
            key.parameter()
                .validate_value(&op, scalar.value)
                .and_then(|()| key.parameter().write(&mut op, scalar.clone()))
                .map_err(|message| (Some(key), message))?;
        }
        crate::numeric_parameter::validate_operation(&op).map_err(|message| {
            let owner = self
                .last_edited
                .filter(|key| self.field(*key).is_some_and(|field| field.active))
                .or(self.focus);
            (owner, message)
        })?;
        Ok(op)
    }

    fn overrides(&self) -> impl Iterator<Item = (ParameterKey, &Scalar)> {
        self.active().filter_map(|field| match &field.status {
            FieldStatus::Valid(scalar) => Some((field.key, scalar)),
            _ => None,
        })
    }
}
