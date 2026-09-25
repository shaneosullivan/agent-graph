//! The help text for every command, compiled in from `docs/cli-help.json` by
//! build.rs. The site's /docs page renders the same file, so they can't drift.

include!(concat!(env!("OUT_DIR"), "/help_text.rs"));

/// The source, for the tests that check it covers every command and option.
pub const JSON: &str = include_str!("../docs/cli-help.json");
