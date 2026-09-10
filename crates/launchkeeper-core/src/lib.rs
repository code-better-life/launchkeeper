//! Launchkeeper core: the task model, SQLite storage, LaunchAgent plist
//! generation, a `launchctl` wrapper and the service layer that combines them.
//!
//! Design contract: `docs/M1-design.md`. Two rules shape everything here —
//! launchd is the only scheduler, and Launchkeeper only ever writes plists
//! that are its own: named `com.launchkeeper.*`, or carrying the
//! `LaunchkeeperManaged` marker that [adoption](crate::Service::adopt) writes
//! after backing the original up (`docs/M3-design.md` §3).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod agents;
pub mod ai;
pub mod env;
pub mod error;
pub mod interpreters;
pub mod launchctl;
pub mod logs;
pub mod paths;
mod plist_gen;
pub mod runner_install;
pub mod service;
pub mod store;
pub mod task;

/// Plist generation. (The implementation lives in `plist_gen` so that the
/// module name does not shadow the `plist` crate inside this crate.)
pub mod plist {
    pub use crate::plist_gen::{
        MANAGED_KEY, PlistOptions, THROTTLE_INTERVAL, build_plist, is_ours, write_plist,
    };
}

pub use crate::agents::{Adoptable, AdoptionPlan, ExternalAgent};
pub use crate::ai::{AiConfig, AiProvider, ExplainInput, Insight, Prompt};
pub use crate::error::{Error, Result};
pub use crate::interpreters::{Detected, Interpreter, InterpreterKind, Origin};
pub use crate::launchctl::JobStatus;
pub use crate::logs::DEFAULT_KEEP_RUNS;
pub use crate::plist::PlistOptions;
pub use crate::service::Service;
pub use crate::store::{Run, RunId, StopReason, Store, TriggerKind};
pub use crate::task::{CalendarEntry, LABEL_PREFIX, Task, TaskName, Trigger};
