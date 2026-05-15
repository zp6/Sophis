use crate::flow_context::FlowContext;
use sophis_core::warn;
use sophis_p2p_lib::{Router, common::ProtocolError};
use sophis_utils::any::type_name_short;
use std::sync::Arc;

#[async_trait::async_trait]
pub trait Flow
where
    Self: 'static + Send + Sync,
{
    fn name(&self) -> &'static str {
        type_name_short::<Self>()
    }

    fn router(&self) -> Option<Arc<Router>>;

    async fn start(&mut self) -> Result<(), ProtocolError>;

    /// Audit/F-12 (Session 10, 2026-05-15): `launch` now takes the
    /// [`FlowContext`] so the protocol-error path can record per-IP
    /// misbehavior scores and promote repeat-offenders to a persistent
    /// ban via `ctx.handle_flow_error`. The `Option` lets unit tests
    /// pass `None` to skip the score plumbing without instantiating a
    /// full FlowContext.
    fn launch(mut self: Box<Self>, ctx: Option<FlowContext>) {
        tokio::spawn(async move {
            let res = self.start().await;
            if let Err(err) = res
                && let Some(router) = self.router()
            {
                // F-12: record + maybe ban BEFORE the connection close.
                // The score manager is in-memory and cheap; the ban path
                // is async and may evict the same router below — the
                // current Router::close() short-circuits if already
                // terminated.
                if let Some(ctx) = ctx.as_ref() {
                    ctx.handle_flow_error(&err, &router).await;
                }
                router.try_sending_reject_message(&err).await;
                if router.close().await || !err.is_connection_closed_error() {
                    warn!("{} flow error: {}, disconnecting from peer {}.", self.name(), err, router);
                }
            }
        });
    }
}
