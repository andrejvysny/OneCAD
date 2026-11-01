//! Create Component form state: steps, gating, configurations and the saved spec. Pure; the
//! only backend traffic is the requests it queues and the replies it is handed.
use super::{
    Op, Request, Scope,
    validate::{
        self, attachment_name_error, configuration_name_error, csv, id_error, name_error,
        version_error,
    },
};
use crate::library_catalog::draft::format_number;
use onecad_app::{
    dto::{LibraryComponentDto, VariableDto},
    library::{AttachmentSpecInput, FreeParameterInput, NewComponentSpec},
};
use onecad_core::{document::record::ComponentParamValue, ids::BodyId};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const STEPS: [&str; 4] = ["Source", "Details", "Parameters", "Attachments"];
pub(crate) const LAST: usize = STEPS.len() - 1;

#[derive(Clone, Debug)]
pub(crate) struct AttachmentRow {
    pub name: String,
    pub input: AttachmentSpecInput,
}

/// One named variant: the text of every exposed parameter's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigRow {
    pub name: String,
    pub values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Saved {
    pub id: String,
    pub version: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Editing,
    /// The next click on the source body names an attachment.
    Picking,
    Resolving,
    Saving,
    Saved(Saved),
}

/// What a reply meant for the host.
#[derive(Debug, PartialEq)]
pub(crate) enum Landed {
    Dropped,
    Picked,
    Saved(Box<LibraryComponentDto>),
    /// The save was refused or failed; the wizard stays open with the reason.
    Refused,
}

#[derive(Clone, Debug)]
pub(crate) struct WizardState {
    pub step: usize,
    pub scope: Scope,
    pub body: BodyId,
    pub body_name: String,
    pub parametric: bool,
    pub name: String,
    pub id: String,
    pub version: String,
    pub category: String,
    pub tags: String,
    pub designation: String,
    pub id_edited: bool,
    pub exposed: BTreeSet<String>,
    pub configs: Vec<ConfigRow>,
    pub default_config: Option<String>,
    pub attachments: Vec<AttachmentRow>,
    /// The name the next picked attachment gets.
    pub new_attachment: String,
    pub phase: Phase,
    pub error: Option<String>,
    /// The body is several solids: offer the union retry.
    pub union_offer: bool,
    pub union_solids: bool,
    next: u64,
    inflight: Option<u64>,
    outbox: Vec<Request>,
}

/// Variables the form can expose: evaluated without error.
pub(crate) fn usable(variables: &[VariableDto]) -> impl Iterator<Item = &VariableDto> {
    variables.iter().filter(|v| v.error.is_none())
}

impl WizardState {
    pub(crate) fn new(scope: Scope, body: BodyId, body_name: String) -> Self {
        let name = body_name.clone();
        Self {
            step: 0,
            scope,
            body,
            id: validate::suggest_id(&name),
            body_name,
            parametric: false,
            name,
            version: "1.0.0".into(),
            category: String::new(),
            tags: String::new(),
            designation: String::new(),
            id_edited: false,
            exposed: BTreeSet::new(),
            configs: Vec::new(),
            default_config: None,
            attachments: Vec::new(),
            new_attachment: "attach_1".into(),
            phase: Phase::Editing,
            error: None,
            union_offer: false,
            union_solids: false,
            next: 0,
            inflight: None,
            outbox: Vec::new(),
        }
    }

    pub(crate) fn busy(&self) -> bool {
        matches!(self.phase, Phase::Resolving | Phase::Saving)
    }

    pub(crate) fn take_outbox(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
    }

    /// The id follows the name until the user types one.
    pub(crate) fn follow_name(&mut self) {
        if !self.id_edited {
            self.id = validate::suggest_id(&self.name);
        }
    }

    // ---- parameters ----

    pub(crate) fn set_exposed(&mut self, variables: &[VariableDto], name: &str, on: bool) {
        if on {
            self.exposed.insert(name.to_owned());
        } else {
            self.exposed.remove(name);
        }
        self.sync_cells(variables);
    }

    /// Every configuration holds one cell per exposed parameter, seeded with its value.
    pub(crate) fn sync_cells(&mut self, variables: &[VariableDto]) {
        let exposed = &self.exposed;
        for config in &mut self.configs {
            config.values.retain(|key, _| exposed.contains(key));
            for key in exposed {
                let seed = usable(variables)
                    .find(|v| v.name == *key)
                    .map_or_else(String::new, |v| format_number(v.resolved_value));
                config.values.entry(key.clone()).or_insert(seed);
            }
        }
    }

    pub(crate) fn add_config(&mut self, variables: &[VariableDto]) {
        let mut n = self.configs.len() + 1;
        while self.configs.iter().any(|c| c.name == format!("Config {n}")) {
            n += 1;
        }
        self.configs.push(ConfigRow {
            name: format!("Config {n}"),
            values: BTreeMap::new(),
        });
        self.sync_cells(variables);
    }

    pub(crate) fn remove_config(&mut self, index: usize) {
        if index < self.configs.len() {
            let removed = self.configs.remove(index);
            if self.default_config.as_deref() == Some(removed.name.as_str()) {
                self.default_config = None;
            }
        }
    }

    /// Renames one configuration, keeping the default selection attached to it.
    pub(crate) fn rename_config(&mut self, index: usize, name: String) {
        let Some(config) = self.configs.get_mut(index) else {
            return;
        };
        if self.default_config.as_deref() == Some(config.name.as_str()) {
            self.default_config = Some(name.clone());
        }
        config.name = name;
    }

    /// The first problem in the configuration table, else `None`.
    pub(crate) fn config_error(&self) -> Option<String> {
        for (i, config) in self.configs.iter().enumerate() {
            let others = self
                .configs
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, c)| c.name.as_str());
            if let Some(error) = configuration_name_error(&config.name, others) {
                return Some(format!("Configuration {}: {error}", i + 1));
            }
            for key in &self.exposed {
                let text = config.values.get(key).map_or("", String::as_str);
                if let Err(error) = validate::number(text) {
                    return Some(format!("{} / {key}: {error}", config.name));
                }
            }
        }
        None
    }

    // ---- gating ----

    /// Why the form cannot leave `step` yet, else `None`.
    pub(crate) fn blocker(&self, step: usize) -> Option<String> {
        match step {
            1 => name_error(&self.name)
                .or_else(|| id_error(&self.id))
                .or_else(|| version_error(&self.version)),
            2 if self.parametric => {
                if self.exposed.is_empty() {
                    Some("Expose at least one variable, or save a static component".into())
                } else {
                    self.config_error()
                }
            }
            _ => None,
        }
    }

    /// Why Save is unavailable now, else `None`; checks every earlier step too.
    pub(crate) fn save_blocker(&self) -> Option<String> {
        (1..=2).find_map(|step| self.blocker(step))
    }

    pub(crate) fn can_advance(&self) -> bool {
        self.phase == Phase::Editing && self.step < LAST && self.blocker(self.step).is_none()
    }

    pub(crate) fn advance(&mut self) {
        if self.can_advance() {
            self.step += 1;
        }
    }

    pub(crate) fn back(&mut self) {
        if self.phase == Phase::Editing {
            self.step = self.step.saturating_sub(1);
        }
    }

    // ---- spec ----

    /// The save request's spec, checked against the document's current variables.
    pub(crate) fn spec(&self, variables: &[VariableDto]) -> Result<NewComponentSpec, String> {
        if let Some(error) = self.save_blocker() {
            return Err(error);
        }
        let mut parameters = BTreeMap::new();
        let mut configurations = BTreeMap::new();
        if self.parametric {
            for key in &self.exposed {
                let variable = usable(variables)
                    .find(|v| v.name == *key)
                    .ok_or_else(|| format!("Variable `{key}` no longer exists"))?;
                parameters.insert(
                    key.clone(),
                    FreeParameterInput {
                        key: key.clone(),
                        value: variable.resolved_value,
                    },
                );
            }
            for config in &self.configs {
                let mut values = BTreeMap::new();
                for (key, text) in &config.values {
                    let n = validate::number(text).map_err(|e| format!("{}: {e}", config.name))?;
                    values.insert(key.clone(), ComponentParamValue::Number(n));
                }
                configurations.insert(config.name.clone(), values);
            }
        }
        let designation = Some(self.designation.trim().to_owned()).filter(|d| !d.is_empty());
        if let Some(template) = &designation {
            unknown_placeholder(template, &parameters)?;
        }
        Ok(NewComponentSpec {
            id: self.id.clone(),
            version: self.version.clone(),
            name: self.name.trim().to_owned(),
            category: csv(&self.category),
            tags: csv(&self.tags),
            designation,
            attachments: self
                .attachments
                .iter()
                .map(|row| (row.name.clone(), row.input.clone()))
                .collect(),
            parameters,
            default_configuration: self
                .default_config
                .clone()
                .filter(|d| configurations.contains_key(d)),
            configurations,
            static_save: !self.parametric,
        })
    }

    // ---- requests ----

    fn queue(&mut self, op: Op) -> u64 {
        self.next += 1;
        self.inflight = Some(self.next);
        self.outbox.push(Request {
            sequence: self.next,
            scope: self.scope.clone(),
            op,
        });
        self.next
    }

    pub(crate) fn begin_pick(&mut self) -> Result<(), String> {
        let name = self.new_attachment.trim().to_owned();
        let others = self.attachments.iter().map(|a| a.name.as_str());
        if let Some(error) = attachment_name_error(&name, others) {
            return Err(error);
        }
        self.new_attachment = name;
        self.error = None;
        self.phase = Phase::Picking;
        Ok(())
    }

    pub(crate) fn cancel_pick(&mut self) {
        if self.phase == Phase::Picking {
            self.phase = Phase::Editing;
        }
    }

    /// The click landed on `topo_key` of the source body under head `snapshot_id`.
    pub(crate) fn resolve_pick(&mut self, topo_key: String, snapshot_id: u64) {
        let op = Op::Pick {
            body_id: self.body.to_string(),
            topo_key,
            name: self.new_attachment.clone(),
            snapshot_id,
        };
        self.queue(op);
        self.phase = Phase::Resolving;
    }

    pub(crate) fn remove_attachment(&mut self, index: usize) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
        }
    }

    pub(crate) fn rename_attachment(&mut self, index: usize, name: &str) -> Result<(), String> {
        let others = self
            .attachments
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != index)
            .map(|(_, a)| a.name.as_str());
        if let Some(error) = attachment_name_error(name, others) {
            return Err(error);
        }
        if let Some(row) = self.attachments.get_mut(index) {
            let kind = row.input.on.split(':').next().unwrap_or("face").to_owned();
            row.name = name.to_owned();
            row.input.on = format!("{kind}:{name}");
        }
        Ok(())
    }

    pub(crate) fn request_save(&mut self, variables: &[VariableDto]) {
        match self.spec(variables) {
            Ok(spec) => {
                self.error = None;
                let op = Op::Save {
                    body_id: self.body.to_string(),
                    spec: Box::new(spec),
                    union_solids: self.union_solids,
                };
                self.queue(op);
                self.phase = Phase::Saving;
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// A request the bridge refused never gets a reply.
    pub(crate) fn refuse(&mut self, current: &Scope, sequence: u64, error: String) -> Landed {
        self.accept(Some(current), sequence, Err(error))
    }

    /// Applies the reply to the request in flight. A save settles whatever document is
    /// current now (it wrote to the library, so the outcome must be reported); a pick
    /// belongs to the document it was asked about and is dropped for any other. A reply
    /// of the wrong kind settles the phase to an error rather than leaving it waiting.
    pub(crate) fn accept(
        &mut self,
        current: Option<&Scope>,
        sequence: u64,
        result: Result<super::Reply, String>,
    ) -> Landed {
        if self.inflight != Some(sequence) {
            return Landed::Dropped;
        }
        self.inflight = None;
        match (&self.phase, result) {
            (Phase::Saving, Ok(super::Reply::Saved(dto))) => {
                self.phase = Phase::Saved(Saved {
                    id: dto.id.clone(),
                    version: dto.version.clone(),
                    name: dto.name.clone(),
                });
                Landed::Saved(dto)
            }
            (Phase::Saving, Err(error)) => {
                self.phase = Phase::Editing;
                self.save_refused(&error);
                Landed::Refused
            }
            (Phase::Resolving, _) if current != Some(&self.scope) => Landed::Dropped,
            (Phase::Resolving, Ok(super::Reply::Picked(input))) => {
                let name = std::mem::take(&mut self.new_attachment);
                self.attachments.push(AttachmentRow {
                    name,
                    input: *input,
                });
                self.new_attachment = format!("attach_{}", self.attachments.len() + 1);
                self.phase = Phase::Editing;
                Landed::Picked
            }
            (Phase::Resolving, Err(error)) => {
                self.phase = Phase::Editing;
                self.error = Some(strip_context(&error));
                Landed::Refused
            }
            (Phase::Saving | Phase::Resolving, Ok(_)) => {
                self.phase = Phase::Editing;
                self.error = Some("The component service sent an unexpected reply".into());
                Landed::Refused
            }
            _ => Landed::Dropped,
        }
    }

    fn save_refused(&mut self, error: &str) {
        self.union_offer = error.contains(onecad_app::library::MULTI_SOLID_REFUSAL);
        self.error = Some(if self.union_offer {
            "This body is several solids, and a component must be one. Union solids fuses them \
             in the saved package only; your document keeps its bodies."
                .to_owned()
        } else if error.contains("already exists") {
            "That id@version already exists — bump the version".to_owned()
        } else {
            strip_context(error)
        });
    }
}

/// `"attachmentFromPick: attachments need…"` -> `"attachments need…"`.
fn strip_context(error: &str) -> String {
    match error.split_once(": ") {
        Some((head, rest)) if !head.contains(' ') => rest.to_owned(),
        _ => error.to_owned(),
    }
}

fn unknown_placeholder(
    template: &str,
    parameters: &BTreeMap<String, FreeParameterInput>,
) -> Result<(), String> {
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            return Err("The designation has an unclosed '{'".into());
        };
        let key = &rest[open + 1..open + close];
        if !parameters.contains_key(key) {
            return Err(format!(
                "The designation uses {{{key}}}, which is not an exposed parameter"
            ));
        }
        rest = &rest[open + close + 1..];
    }
    Ok(())
}
