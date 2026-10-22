use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::BoundedText;

// Capture protocol context, not arbitrary application or credential fields.
#[derive(Default)]
pub(crate) struct Fields(BTreeMap<&'static str, String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if !matches!(
            field.name(),
            "id" | "space" | "pn" | "path_id" | "scope" | "timer" | "ty" | "path"
        ) {
            return;
        }
        let mut text = BoundedText::new(256);
        let _ = write!(text, "{value:?}");
        self.0.insert(field.name(), text.finish());
    }
}

pub(crate) fn render<S>(event: &Event<'_>, context: Context<'_, S>) -> String
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    let Some(scope) = context.event_scope(event) else {
        return String::new();
    };
    let mut text = BoundedText::new(2048);
    let mut count = 0;
    for span in scope.from_root() {
        let extensions = span.extensions();
        let Some(fields) = extensions.get::<Fields>() else {
            continue;
        };
        if count == 8 {
            let _ = text.write_str(" [truncated]");
            break;
        }
        if count > 0 {
            let _ = text.write_str("/");
        }
        count += 1;
        let _ = write!(text, "{}{{", span.name());
        for (index, (name, value)) in fields.0.iter().enumerate() {
            if index > 0 {
                let _ = text.write_str(" ");
            }
            let _ = write!(text, "{name}={value}");
        }
        let _ = text.write_str("}");
    }
    text.finish()
}
