//! Collect task cleanup failures without skipping the remaining owners.
use tokio::task::JoinError;

#[derive(Default)]
pub(crate) struct Errors(Vec<String>);

impl Errors {
    pub(crate) fn join(&mut self, owner: &str, result: Result<(), JoinError>, cancelled: bool) {
        if let Err(error) = result
            && !(cancelled && error.is_cancelled())
        {
            self.0.push(format!("{owner}: {error}"));
        }
    }

    pub(crate) fn result(&mut self, owner: &str, result: Result<(), String>) {
        if let Err(error) = result {
            self.0.push(format!("{owner}: {error}"));
        }
    }

    pub(crate) fn finish(self) -> Result<(), String> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(self.0.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn deliberate_abort_is_allowed_but_panic_and_unexpected_cancel_survive() {
        let task = tokio::spawn(std::future::pending::<()>());
        task.abort();
        let mut errors = Errors::default();
        errors.join("deliberate", task.await, true);
        assert!(errors.finish().is_ok());
        let task = tokio::spawn(std::future::pending::<()>());
        task.abort();
        let mut errors = Errors::default();
        errors.join("unexpected", task.await, false);
        let panic = tokio::spawn(async {
            panic!("cleanup panic");
        });
        errors.join("panic", panic.await, true);
        let error = errors.finish().unwrap_err();
        assert!(error.contains("unexpected"));
        assert!(error.contains("panic"));
    }
}
