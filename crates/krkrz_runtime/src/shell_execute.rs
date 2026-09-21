//! Desktop operations are provided by the native host, never by headless runs.
use crate::Services;
use anyhow::Result;

type Handler = Box<dyn FnMut(&str, &str) -> Result<bool>>;

#[derive(Default)]
pub(crate) struct State {
    handler: Option<Handler>,
}

impl Services {
    /// Return whether the OS accepted the launch request. Errors and rejected
    /// requests become false, matching System.shellExecute's native contract.
    pub fn set_shell_execute_handler<F>(&mut self, handler: F)
    where
        F: FnMut(&str, &str) -> Result<bool> + 'static,
    {
        self.shell_execute.handler = Some(Box::new(handler));
    }

    pub(crate) fn shell_execute(&mut self, target: &str, parameters: &str) -> bool {
        let Some(handler) = self.shell_execute.handler.as_mut() else {
            self.messages
                .push("System.shellExecute: no desktop host is installed".into());
            return false;
        };
        match handler(target, parameters) {
            Ok(success) => success,
            Err(error) => {
                self.messages
                    .push(format!("System.shellExecute: {error:#}"));
                false
            }
        }
    }
}
