//! Menu events retain a registration token so reloads cannot retarget queued clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuLocation {
    Export,
    Extensions,
}

pub(super) struct ExtensionRoute {
    pub token: u64,
    pub command: String,
    pub title: String,
    pub location: MenuLocation,
}

#[derive(Default)]
pub(super) struct ExtensionRoutes {
    next: u64,
    generation: u64,
    pub entries: Vec<ExtensionRoute>,
}

impl ExtensionRoutes {
    pub fn replace(
        &mut self,
        export: &[(String, String)],
        generic: &[(String, String)],
        generation: u64,
    ) -> bool {
        let commands = export
            .iter()
            .map(|c| (MenuLocation::Export, c))
            .chain(generic.iter().map(|c| (MenuLocation::Extensions, c)));
        if self.generation == generation
            && self.entries.len() == export.len() + generic.len()
            && self
                .entries
                .iter()
                .zip(commands.clone())
                .all(|(entry, (location, command))| {
                    entry.location == location
                        && entry.command == command.0
                        && entry.title == command.1
                })
        {
            return false;
        }
        self.generation = generation;
        self.entries.clear();
        for (location, (command, title)) in commands {
            let Some(token) = self.next.checked_add(1) else {
                break;
            };
            self.next = token;
            self.entries.push(ExtensionRoute {
                token,
                command: command.clone(),
                title: title.clone(),
                location,
            });
        }
        true
    }

    pub fn command(&self, token: u64) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.token == token)
            .map(|entry| entry.command.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(id: &str) -> Vec<(String, String)> {
        vec![(id.into(), "Run".into())]
    }

    #[test]
    fn generic_commands_have_a_distinct_menu_route() {
        let mut routes = ExtensionRoutes::default();
        routes.replace(&command("export.send"), &command("generic.run"), 1);
        assert_eq!(routes.entries[0].location, MenuLocation::Export);
        assert_eq!(routes.entries[1].location, MenuLocation::Extensions);
        assert_ne!(routes.entries[0].token, routes.entries[1].token);
        assert_eq!(
            routes.command(routes.entries[1].token).as_deref(),
            Some("generic.run")
        );
    }

    #[test]
    fn queued_click_cannot_target_another_extension_after_reload() {
        let mut routes = ExtensionRoutes::default();
        routes.replace(&command("first.send"), &[], 1);
        let old = routes.entries[0].token;
        routes.replace(&[], &command("second.send"), 2);
        assert_eq!(routes.command(old), None);
        assert_eq!(
            routes.command(routes.entries[0].token).as_deref(),
            Some("second.send")
        );
        routes.replace(&[], &[], 3);
        assert!(routes.command(old).is_none());
    }

    #[test]
    fn unchanged_commands_keep_their_registration() {
        let mut routes = ExtensionRoutes::default();
        let commands = command("addon.send");
        assert!(routes.replace(&[], &commands, 1));
        let token = routes.entries[0].token;
        assert!(!routes.replace(&[], &commands, 1));
        assert_eq!(routes.command(token).as_deref(), Some("addon.send"));
    }

    #[test]
    fn moving_command_between_menus_invalidates_queued_event() {
        let mut routes = ExtensionRoutes::default();
        let commands = command("addon.send");
        routes.replace(&commands, &[], 1);
        let token = routes.entries[0].token;
        assert!(routes.replace(&[], &commands, 1));
        assert_eq!(routes.command(token), None);
        assert_eq!(routes.entries[0].location, MenuLocation::Extensions);
    }

    #[test]
    fn same_command_reinstalled_invalidates_old_menu_event() {
        let mut routes = ExtensionRoutes::default();
        let commands = command("addon.send");
        routes.replace(&[], &commands, 1);
        let token = routes.entries[0].token;
        assert!(routes.replace(&[], &commands, 2));
        assert_eq!(routes.command(token), None);
    }
}
