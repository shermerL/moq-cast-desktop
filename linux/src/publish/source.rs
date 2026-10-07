//! Capture choices scoped to the current desktop session.

/// A source selected before publishing starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureSource {
    /// Let the Wayland portal select a screen.
    Portal,
    /// Let the Wayland portal select a single window.
    PortalWindow,
    /// Capture an enumerated X11 display.
    Display {
        id: String,
        name: String,
        width: u32,
        height: u32,
    },
}

impl CaptureSource {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Portal => "Screen (system picker)".into(),
            Self::PortalWindow => "Window (system picker)".into(),
            Self::Display {
                name,
                width,
                height,
                ..
            } => format!("{name} · {width} × {height}"),
        }
    }

    /// Resolve a refreshed choice without silently selecting a different display.
    pub(crate) fn resolve(&self, choices: &[Self]) -> Option<Self> {
        choices.iter().find(|candidate| self == *candidate).cloned()
    }
}

/// Available sources or the current enumeration state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SourceCatalog {
    /// Sources have not been requested yet.
    #[default]
    Unloaded,
    /// Source enumeration is in progress.
    Loading,
    /// The system portal owns source selection.
    Portal,
    /// Explicit screen choices in the X11 session.
    Ready(Vec<CaptureSource>),
    /// Source enumeration failed.
    Failed(String),
}

impl SourceCatalog {
    #[cfg(any(target_os = "linux", test))]
    fn resolve(&self, choice: &CaptureSource) -> Option<CaptureSource> {
        match (self, choice) {
            (Self::Portal, CaptureSource::Portal | CaptureSource::PortalWindow) => {
                Some(choice.clone())
            }
            (Self::Ready(choices), _) => choice.resolve(choices),
            _ => None,
        }
    }

    pub(crate) fn selected(&self, choice: Option<&CaptureSource>) -> Option<CaptureSource> {
        match self {
            Self::Portal => match choice {
                None => Some(CaptureSource::Portal),
                Some(choice) => match choice {
                    CaptureSource::Portal | CaptureSource::PortalWindow => Some(choice.clone()),
                    _ => None,
                },
            },
            Self::Ready(choices) => choice.and_then(|choice| choice.resolve(choices)),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Session {
    X11,
    Wayland,
}

fn session(kind: Option<&str>, wayland: bool, x11: bool) -> Result<Session, &'static str> {
    match kind {
        Some("x11") => Ok(Session::X11),
        Some("wayland") => Ok(Session::Wayland),
        Some(kind) if !kind.is_empty() => {
            Err("Screen sharing requires an X11 or Wayland desktop session.")
        }
        _ if wayland => Ok(Session::Wayland),
        _ if x11 => Ok(Session::X11),
        _ => Err("No X11 or Wayland desktop session is available."),
    }
}

pub(crate) async fn enumerate() -> Result<SourceCatalog, String> {
    match session(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
    )? {
        Session::Wayland => Ok(SourceCatalog::Portal),
        Session::X11 => enumerate_x11().await.map(SourceCatalog::Ready),
    }
}

#[cfg(target_os = "linux")]
async fn enumerate_x11() -> Result<Vec<CaptureSource>, String> {
    Ok(moq_video::capture::displays()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|display| CaptureSource::Display {
            id: display.id,
            name: display.name,
            width: display.width,
            height: display.height,
        })
        .collect())
}

#[cfg(not(target_os = "linux"))]
async fn enumerate_x11() -> Result<Vec<CaptureSource>, String> {
    Err("X11 source selection is available only on Linux.".into())
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct Unavailable(pub(crate) String);

#[cfg(target_os = "linux")]
pub(crate) async fn prepare(
    choice: CaptureSource,
) -> Result<moq_video::capture::Source, Unavailable> {
    let current = enumerate().await.map_err(Unavailable)?;
    let choice = current.resolve(&choice).ok_or_else(|| {
        Unavailable(
            "The capture source is no longer available or has changed. Refresh and select it again."
                .into(),
        )
    })?;
    Ok(match choice {
        CaptureSource::Portal | CaptureSource::PortalWindow => {
            use moq_video::capture::portal::{Kind, Selection};
            let kind = if choice == CaptureSource::PortalWindow {
                Kind::Window
            } else {
                Kind::Screen
            };
            // Called once per user-started publication. Capture config clones retain
            // this grant across demand-driven reopens, while the next Start reselects.
            moq_video::capture::Source::Portal(Selection::new(kind))
        }
        CaptureSource::Display { id, .. } => moq_video::capture::Source::Display(Some(id)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_session_takes_precedence_over_inherited_display_variables() {
        assert_eq!(session(Some("x11"), true, true), Ok(Session::X11));
        assert_eq!(session(Some("wayland"), false, true), Ok(Session::Wayland));
        assert_eq!(session(None, true, true), Ok(Session::Wayland));
        assert_eq!(session(None, false, true), Ok(Session::X11));
        assert!(session(None, false, false).is_err());
        assert!(session(Some("tty"), false, true).is_err());
    }

    #[test]
    fn display_index_reuse_after_layout_change_requires_reselection() {
        let chosen = CaptureSource::Display {
            id: "x11:0".into(),
            name: "DP-1".into(),
            width: 1920,
            height: 1080,
        };
        let moved = CaptureSource::Display {
            id: "x11:0".into(),
            name: "HDMI-1".into(),
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            chosen.resolve(std::slice::from_ref(&chosen)),
            Some(chosen.clone())
        );
        assert_eq!(chosen.resolve(&[moved]), None);
    }

    #[test]
    fn session_change_never_substitutes_portal_for_an_x11_choice() {
        let chosen = CaptureSource::Display {
            id: "x11:0".into(),
            name: "DP-1".into(),
            width: 1920,
            height: 1080,
        };
        assert_eq!(SourceCatalog::Portal.resolve(&chosen), None);
        assert_eq!(
            SourceCatalog::Ready(vec![chosen]).resolve(&CaptureSource::Portal),
            None
        );
        assert_eq!(
            SourceCatalog::Portal.resolve(&CaptureSource::Portal),
            Some(CaptureSource::Portal)
        );
    }

    #[test]
    fn window_choice_survives_refresh_but_not_a_session_change() {
        let window = CaptureSource::PortalWindow;
        assert_eq!(SourceCatalog::Portal.resolve(&window), Some(window.clone()));
        assert_eq!(
            SourceCatalog::Portal.selected(Some(&window)),
            Some(window.clone())
        );
        assert_eq!(SourceCatalog::Ready(vec![]).resolve(&window), None);
        let x11 = CaptureSource::Display {
            id: "x11:0".into(),
            name: "screen".into(),
            width: 1920,
            height: 1080,
        };
        assert_eq!(SourceCatalog::Portal.selected(Some(&x11)), None);
    }

    #[test]
    fn missing_and_failed_catalogs_never_fall_back_to_a_different_source() {
        assert_eq!(SourceCatalog::Unloaded.selected(None), None);
        assert_eq!(SourceCatalog::Loading.selected(None), None);
        assert_eq!(
            SourceCatalog::Failed("unavailable".into()).selected(None),
            None
        );
        assert_eq!(SourceCatalog::Ready(vec![]).selected(None), None);
        assert_eq!(
            SourceCatalog::Portal.selected(None),
            Some(CaptureSource::Portal)
        );
    }
}
