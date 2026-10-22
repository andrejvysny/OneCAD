use crate::{api, package::MAX_COMPONENT_BYTES, HostError, Result as HostResult};
use wasmtime::{
    component::{Component, Linker},
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
};

wasmtime::component::bindgen!({
    path: "../onecad-extension-api/wit",
    world: "addon",
});

#[derive(Debug, Clone, Copy)]
pub struct SandboxLimits {
    pub fuel: u64,
    pub memory_bytes: usize,
    pub max_response_bytes: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            fuel: 5_000_000,
            memory_bytes: 32 * 1024 * 1024,
            max_response_bytes: 64 * 1024,
        }
    }
}

pub(crate) struct Sandbox {
    engine: Engine,
    limits: SandboxLimits,
}

impl Sandbox {
    pub fn new(limits: SandboxLimits) -> HostResult<Self> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .max_wasm_stack(512 * 1024);
        let engine = Engine::new(&config).map_err(failed)?;
        Ok(Self { engine, limits })
    }

    pub fn validate(&self, bytes: &[u8]) -> HostResult<()> {
        let component = self.compile(bytes)?;
        let mut store = self.store()?;
        Addon::instantiate(&mut store, &component, &Linker::new(&self.engine)).map_err(failed)?;
        Ok(())
    }

    fn compile(&self, bytes: &[u8]) -> HostResult<Component> {
        if bytes.len() > MAX_COMPONENT_BYTES || !bytes.starts_with(b"\0asm") {
            return Err(HostError::InvalidPackage("invalid component binary".into()));
        }
        // Never deserialize native/precompiled artifacts from an untrusted package.
        let component = Component::from_binary(&self.engine, bytes).map_err(failed)?;
        if component
            .component_type()
            .imports(&self.engine)
            .next()
            .is_some()
        {
            return Err(HostError::PermissionDenied(
                "component imports are forbidden (including WASI)".into(),
            ));
        }
        Ok(component)
    }

    pub fn execute(
        &self,
        bytes: &[u8],
        command: &str,
        context: &api::CommandContext,
    ) -> HostResult<api::CommandExecution> {
        let component = self.compile(bytes)?;
        let mut store = self.store()?;
        // An empty linker intentionally provides no filesystem, network, clock,
        // environment, process, document or other ambient host access.
        let linker = Linker::new(&self.engine);
        let addon = Addon::instantiate(&mut store, &component, &linker).map_err(failed)?;
        let context = serde_json::to_string(context)?;
        let response = addon
            .call_execute(&mut store, command, &context)
            .map_err(failed)?;
        let text = match &response {
            Ok(text) | Err(text) => text,
        };
        if text.len() > self.limits.max_response_bytes {
            return Err(HostError::Sandbox("command response exceeds limit".into()));
        }
        let response = response.map_err(HostError::Sandbox)?;
        Ok(serde_json::from_str(&response)?)
    }

    fn store(&self) -> HostResult<Store<StoreLimits>> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.memory_bytes)
            .memories(1)
            .instances(16)
            .tables(4)
            .table_elements(10_000)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|limits: &mut StoreLimits| limits);
        store.set_fuel(self.limits.fuel).map_err(failed)?;
        Ok(store)
    }
}

fn failed(error: impl std::fmt::Display) -> HostError {
    HostError::Sandbox(error.to_string())
}
