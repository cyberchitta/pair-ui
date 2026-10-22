//! A moniker names something a page shows: `<scheme>://<project><rest>`. The
//! scheme names the app that minted it, the project which page shows it, and
//! the rest is the app's, carried untouched. A page mints the whole form
//! because it cannot know which host it runs under; pair-ui routes by the
//! project alone, and leaves the scheme to a host that runs many apps.

use std::fmt;
use std::str::FromStr;

/// A name an app minted for something its page shows. One token, with no
/// whitespace or control characters, so it pastes into a chat whole. Only
/// parsing makes one.
///
/// ```
/// use pair_ui::{BadMoniker, Moniker};
/// let m: Moniker = "khana://sorted-studs/joint#part=upper".parse().unwrap();
/// assert_eq!((m.scheme(), m.project()), ("khana", "sorted-studs"));
/// assert!(matches!("/a/path".parse::<Moniker>(), Err(BadMoniker::NotAMoniker(_))));
/// assert!(matches!("khana://p/a b".parse::<Moniker>(), Err(BadMoniker::Malformed(_))));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moniker {
    scheme: String,
    project: String,
    /// Everything after the project, verbatim: empty, or opening with `/`,
    /// `?` or `#`.
    rest: String,
}

/// Why text is not a moniker. A caller that also takes other kinds of name
/// (a path, say) tries those on [`BadMoniker::NotAMoniker`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BadMoniker {
    #[error("not a moniker (<scheme>://<project>…): {0:?}")]
    NotAMoniker(String),
    #[error("not a moniker: opens with <scheme>:// but the rest is malformed: {0:?}")]
    Malformed(String),
}

impl Moniker {
    /// The app that minted it: lowercase, `[a-z][a-z0-9+.-]*`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The project whose page shows it.
    pub fn project(&self) -> &str {
        &self.project
    }
}

impl FromStr for Moniker {
    type Err = BadMoniker;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (scheme, after) = s
            .split_once("://")
            .filter(|(scheme, _)| valid_scheme(scheme))
            .ok_or_else(|| BadMoniker::NotAMoniker(s.to_string()))?;
        let (project, rest) = after.split_at(after.find(['/', '?', '#']).unwrap_or(after.len()));
        let clean = !after.chars().any(|c| c.is_whitespace() || c.is_control());
        if !clean || project.is_empty() || project == "." || project == ".." {
            return Err(BadMoniker::Malformed(s.to_string()));
        }
        Ok(Moniker {
            scheme: scheme.to_string(),
            project: project.to_string(),
            rest: rest.to_string(),
        })
    }
}

fn valid_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+.-".contains(c))
}

impl fmt::Display for Moniker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}{}", self.scheme, self.project, self.rest)
    }
}
