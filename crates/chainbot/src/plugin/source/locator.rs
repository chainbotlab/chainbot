use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PluginSourceKind {
    GitHub,
    Git,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginSourceLocator {
    GitHub {
        owner: String,
        repo: String,
        git_ref: Option<String>,
    },
    Git {
        remote: String,
        git_ref: Option<String>,
    },
}

impl PluginSourceLocator {
    pub(crate) fn kind(&self) -> PluginSourceKind {
        match self {
            Self::GitHub { .. } => PluginSourceKind::GitHub,
            Self::Git { .. } => PluginSourceKind::Git,
        }
    }

    pub(crate) fn requested_ref(&self) -> Option<&str> {
        match self {
            Self::GitHub { git_ref, .. } | Self::Git { git_ref, .. } => git_ref.as_deref(),
        }
    }

    pub(crate) fn target(&self) -> String {
        match self {
            Self::GitHub { owner, repo, .. } => format!("{owner}/{repo}"),
            Self::Git { remote, .. } => remote.clone(),
        }
    }

    pub(crate) fn display_label(&self) -> String {
        match self {
            Self::GitHub { owner, repo, git_ref } => {
                render_locator("github", &format!("{owner}/{repo}"), git_ref.as_deref())
            }
            Self::Git { remote, git_ref } => render_locator("git", remote, git_ref.as_deref()),
        }
    }
}

fn render_locator(kind: &str, target: &str, git_ref: Option<&str>) -> String {
    match git_ref {
        Some(value) => format!("{kind} {target} @ {value}"),
        None => format!("{kind} {target}"),
    }
}

impl PluginSourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::Git => "git",
        }
    }
}

impl Display for PluginSourceKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
