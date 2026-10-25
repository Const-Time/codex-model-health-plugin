pub mod domain;
pub mod engine;
mod host;
mod management;
mod schedule;
mod updates;

use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    client::{AuthorError, ComposedPlugin, PluginBuilder},
};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

pub(crate) struct Service {
    engine: engine::Engine,
    state: Mutex<()>,
    execution: Semaphore,
    updates: updates::UpdateChecker,
}
pub fn plugin(engine: engine::Engine) -> Result<ComposedPlugin, AuthorError> {
    let service = Arc::new(Service {
        engine,
        state: Mutex::new(()),
        execution: Semaphore::new(1),
        updates: updates::UpdateChecker::default(),
    });
    let scheduled = Arc::clone(&service);
    PluginBuilder::from_json(include_bytes!("../../plugin.json"))?
        .on(
            gateway_plugin_sdk::client::methods::STATE_MIGRATE,
            |call| async move {
                Ok(gateway_plugin_sdk::client::TypedReply::new(
                    schedule::migrate(call.request)?,
                ))
            },
        )?
        .on(
            gateway_plugin_sdk::client::methods::RECONCILE,
            move |call| {
                let service = Arc::clone(&scheduled);
                async move {
                    schedule::tick(&service, &call.host).await?;
                    Ok(gateway_plugin_sdk::client::TypedReply::new(
                        gateway_plugin_sdk::client::Empty {},
                    ))
                }
            },
        )?
        .management(management::registration(), move |call| {
            let service = Arc::clone(&service);
            async move { management::handle(&service, call).await }
        })?
        .build()
}
pub(crate) fn invalid(message: impl Into<String>) -> PluginFault {
    PluginFault::new(ErrorCode::InvalidInput, message)
}
pub(crate) fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_nanos()
}
pub(crate) fn now_ms() -> u64 {
    (now_nanos() / 1_000_000) as u64
}
