//! A failed startup must finish stopping its instance before another can start.
use super::*;
use crate::worker::host::Launched;

pub(super) async fn initialize(
    shared: &Shared,
    launched: Launched,
) -> Result<(Box<dyn WorkerInstance>, Arc<ProtocolClient>), String> {
    let Launched {
        reader,
        writer,
        mut instance,
        expected,
    } = launched;
    let startup = async {
        let client = ProtocolClient::connect(reader, writer)
            .await
            .map_err(|error| format!("handshake: {error}"))?;
        verify_solver_policy(client.hello().solver_policy_version)?;
        if let Some(expected) = expected {
            verify_hello_axes(&expected, client.hello())?;
        }
        // Consume previous-launch evidence before OpenSession journals new work.
        shared.note_fingerprint(&client.hello().occt.fingerprint);
        shared.seed_poison(shared.host.poison_seeds(&client.hello().occt.fingerprint));
        if shared.config.auto_open_session {
            auto_open_session(shared, &client)
                .await
                .map_err(|error| format!("OpenSession: {error}"))?;
        }
        Ok(Arc::new(client))
    };
    let result = tokio::select! {
        biased;
        () = shared.retired() => Err("retired during startup".into()),
        result = tokio::time::timeout(shared.config.ping_timeout, startup) => {
            result.unwrap_or_else(|_| Err("worker startup deadline exceeded".into()))
        }
    };
    match result {
        Ok(client) => Ok((instance, client)),
        Err(reason) => {
            if instance.force_kill().await == KillOutcome::Abandoned {
                let reason = format!("relaunch-required: in-process startup abandoned ({reason})");
                if !shared.is_retired() {
                    shared.fail(reason.clone());
                }
                return Err(reason);
            }
            Err(reason)
        }
    }
}

#[cfg(test)]
mod tests;
