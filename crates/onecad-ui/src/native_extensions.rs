//! Native extension management. Guests and filesystem work run on the bounded service lane.
mod common;
mod consent;
mod detail;
mod installed;
#[cfg(test)]
mod interaction_tests;
mod manager;
mod menu_surface;
mod palette;
mod service;
mod view;
#[cfg(test)]
mod visual_tests;

use crate::app::App;
use onecad_core::ids::BodyId;
use onecad_extension_api::{CommandContribution, CommandMenu};
use onecad_extension_host::{Approval, PackageSummary, RegisteredCommand};
use std::{collections::BTreeMap, path::PathBuf};

pub(crate) use menu_surface::show as menu;
pub(crate) use palette::route_shortcut;
pub(crate) use service::execute;
pub(crate) use view::{render, show};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ManagerTab {
    Discover,
    #[default]
    Installed,
    Updates,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DetailTab {
    #[default]
    Overview,
    Settings,
    Changelog,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PresentationContext {
    pub idle: bool,
    pub document: bool,
    pub selection: bool,
}

#[derive(Default)]
pub(crate) struct ViewAction {
    pub request: Option<Request>,
    pub command: Option<String>,
    pub page: Option<crate::ui_state::StartPage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Refresh,
    InstallFolder,
    UpdateFolder {
        id: String,
        fingerprint: String,
    },
    ResetApplications,
    Uninstall {
        id: String,
    },
    SetEnabled {
        id: String,
        enabled: bool,
        approval: Option<Approval>,
    },
    SetApplication {
        extension: String,
        application: String,
        executable: Option<PathBuf>,
    },
    Execute {
        command: String,
        fingerprint: String,
        target: Option<onecad_app::api::PreviewContext>,
        selection: Vec<BodyId>,
    },
}

#[derive(Debug)]
pub struct Reply {
    pub generation: u64,
    pub packages: Vec<PackageSummary>,
    pub commands: Vec<RegisteredCommand>,
    pub applications: BTreeMap<String, PathBuf>,
    pub status: String,
    pub warning: bool,
    pub discovery_errors: Vec<String>,
    /// Set only after a successful install/update; discovery changes do not imply success.
    pub review_package: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub tab: ManagerTab,
    pub search: String,
    pub category: usize,
    pub detail: Option<String>,
    pub detail_tab: DetailTab,
    pub developer_open: bool,
    pub reset_confirmation: bool,
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_focus: bool,
    pub generation: u64,
    pub packages: Vec<PackageSummary>,
    pub commands: Vec<RegisteredCommand>,
    pub applications: BTreeMap<String, PathBuf>,
    pub application_drafts: BTreeMap<String, String>,
    pub loaded: bool,
    pub attempted: bool,
    pub pending: bool,
    pub pending_request: Option<Request>,
    pub status: String,
    pub status_warning: bool,
    pub review: Option<PackageSummary>,
    pub discovery_errors: Vec<String>,
}

impl State {
    pub(crate) fn open_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_focus = true;
    }
    pub fn receive(&mut self, reply: Reply) {
        if reply.generation < self.generation {
            return;
        }
        self.generation = reply.generation;
        let review_package = reply
            .review_package
            .as_ref()
            .and_then(|id| {
                reply
                    .packages
                    .iter()
                    .find(|package| &package.manifest.id == id)
            })
            .cloned();
        self.review = self.review.take().and_then(|review| {
            reply
                .packages
                .iter()
                .find(|package| {
                    package.manifest.id == review.manifest.id
                        && package.fingerprint == review.fingerprint
                })
                .cloned()
        });
        self.update_drafts(&reply);
        self.packages = reply.packages;
        self.commands = reply.commands;
        self.applications = reply.applications;
        self.status = reply.status;
        self.status_warning = reply.warning;
        self.discovery_errors = reply.discovery_errors;
        self.loaded = true;
        self.pending = false;
        self.pending_request = None;
        if let Some(package) = review_package {
            self.tab = ManagerTab::Installed;
            self.review = Some(package);
        }
    }

    pub fn failed(&mut self, error: String) {
        if self.pending {
            self.pending = false;
            self.status = error;
            self.status_warning = true;
            self.pending_request = None;
            for entry in &mut self.commands {
                entry.enabled = false;
            }
        }
    }

    fn update_drafts(&mut self, reply: &Reply) {
        let previous = std::mem::take(&mut self.application_drafts);
        for package in &reply.packages {
            for application in &package.manifest.applications {
                let key = application_key(&package.manifest.id, &application.id);
                let old = self
                    .applications
                    .get(&key)
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let saved = reply
                    .applications
                    .get(&key)
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let explicit_reset = matches!(
                    self.pending_request,
                    Some(Request::ResetApplications)
                ) || matches!(self.pending_request.as_ref(), Some(Request::SetApplication { extension, application: selected, executable: None }) if extension == &package.manifest.id && selected == &application.id);
                let draft = previous
                    .get(&key)
                    .filter(|draft| !explicit_reset && **draft != old && old == saved)
                    .cloned()
                    .unwrap_or(saved);
                self.application_drafts.insert(key, draft);
            }
        }
    }
}

pub(crate) fn application_key(extension: &str, application: &str) -> String {
    format!("{extension}/{application}")
}

pub(crate) fn export_command(command: &CommandContribution) -> bool {
    command.menu == Some(CommandMenu::Export) || command.locations.iter().any(|s| s == "export")
}

pub(crate) fn menu_command(command: &CommandContribution) -> bool {
    !export_command(command) && command.locations.iter().any(|location| location == "menu")
}

pub(crate) fn command_available(
    command: &CommandContribution,
    document: bool,
    selection: bool,
) -> bool {
    (!command.requires_document || document)
        && (!command.requires_selection || document && selection)
}

impl App {
    pub fn extension_request(&mut self, request: Request) {
        let was_running = self.doc.documents.running;
        let pending_request = request.clone();
        self.document_request(crate::document_flow::Request::Extension(request));
        if !was_running && self.doc.documents.running {
            let state = &mut self.ui.extensions.native;
            state.attempted = true;
            state.pending = true;
            state.pending_request = Some(pending_request);
            state.status_warning = false;
            state.status = "Extension action pending…".into();
        }
    }

    pub fn run_extension_command(&mut self, command: &str) {
        let fingerprint = self
            .ui
            .extensions
            .native
            .commands
            .iter()
            .find(|entry| entry.command.id == command)
            .and_then(|entry| {
                self.ui
                    .extensions
                    .native
                    .packages
                    .iter()
                    .find(|package| package.manifest.id == entry.extension_id)
            })
            .map(|package| package.fingerprint.clone());
        let Some(fingerprint) = fingerprint else {
            self.doc.status =
                "Extension command is no longer registered. Refresh extensions and try again."
                    .into();
            self.doc.command_status = true;
            return;
        };
        let target = self
            .doc
            .display
            .as_ref()
            .filter(|_| self.doc.bridge.valid())
            .map(|display| display.context.clone());
        let mut selection = self
            .doc
            .selection
            .items()
            .iter()
            .filter_map(|item| item.body())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if target.is_none() {
            selection.clear();
        }
        self.extension_request(Request::Execute {
            command: command.into(),
            fingerprint,
            target,
            selection,
        });
    }

    pub(crate) fn sync_extension_menu(&self) {
        if let Some(menu) = &self.menu {
            let commands = self
                .ui
                .extensions
                .native
                .commands
                .iter()
                .filter(|entry| entry.enabled && export_command(&entry.command))
                .filter(|entry| {
                    command_available(
                        &entry.command,
                        crate::bars::export_allowed(self),
                        self.doc
                            .selection
                            .items()
                            .iter()
                            .any(|item| item.body().is_some()),
                    )
                })
                .map(|entry| (entry.command.id.clone(), entry.command.title.clone()))
                .collect::<Vec<_>>();
            let generic = self
                .ui
                .extensions
                .native
                .commands
                .iter()
                .filter(|entry| entry.enabled && menu_command(&entry.command))
                .filter(|entry| {
                    command_available(
                        &entry.command,
                        crate::bars::export_allowed(self),
                        self.doc
                            .selection
                            .items()
                            .iter()
                            .any(|item| item.body().is_some()),
                    )
                })
                .map(|entry| (entry.command.id.clone(), entry.command.title.clone()))
                .collect::<Vec<_>>();
            menu.set_extension_commands(
                &commands,
                &generic,
                !self.busy(),
                self.ui.extensions.native.generation,
            );
        }
    }
}

#[cfg(test)]
mod state_tests;
