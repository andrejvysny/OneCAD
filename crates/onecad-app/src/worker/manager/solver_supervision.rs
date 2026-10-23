//! Ordinary solver requests include dispatch and mailbox time in their budget.
use super::*;
use onecad_protocol::messages::RespFrame;

pub(super) struct ExpiredSolver {
    pub epoch: u64,
    pub verb: String,
    pub age_ms: u64,
}

impl Shared {
    pub(super) async fn solver_round_trip(
        &self,
        client: &Arc<ProtocolClient>,
        verb: &str,
        args: Value,
    ) -> Result<(RespFrame, Vec<u8>), EngineError> {
        let epoch = self.connection.read().unwrap().epoch;
        let budget = self.config.wedge_deadline(verb);
        let request = async {
            client
                .start_request(verb, args, Lane::Control)
                .await?
                .response_with_bin()
                .await
        };
        match tokio::time::timeout(budget, request).await {
            Ok(result) => {
                let connection = self.connection.read().unwrap();
                if connection.epoch != epoch
                    || connection.state != WorkerState::Ready
                    || self.is_retired()
                    || !connection
                        .client
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, client))
                {
                    return Err(protocol(
                        "Solver response belongs to a replaced worker session",
                    ));
                }
                result.map_err(protocol_err)
            }
            Err(_) => {
                self.expire_solver(client, epoch, verb, budget);
                Err(protocol(&format!(
                    "{verb} deadline exceeded; completion unknown; request not retried"
                )))
            }
        }
    }

    fn expire_solver(
        &self,
        client: &Arc<ProtocolClient>,
        epoch: u64,
        verb: &str,
        budget: Duration,
    ) {
        let mut connection = self.connection.write().unwrap();
        if self.is_retired()
            || connection.epoch != epoch
            || connection.state != WorkerState::Ready
            || !connection
                .client
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, client))
        {
            return;
        }
        connection.expired_solver = Some(ExpiredSolver {
            epoch,
            verb: verb.to_string(),
            age_ms: u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
        });
        connection.state = WorkerState::Restarting;
        self.solver_deadline_signal.notify_one();
    }

    pub(super) fn take_expired_solver(&self, client: &ProtocolClient) -> Option<ExpiredSolver> {
        let mut connection = self.connection.write().unwrap();
        let expired = connection.expired_solver.take()?;
        (connection.epoch == expired.epoch
            && connection
                .client
                .as_ref()
                .is_some_and(|current| std::ptr::eq(current.as_ref(), client)))
        .then_some(expired)
    }
}

#[cfg(test)]
mod tests;
