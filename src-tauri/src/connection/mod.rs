// How Claude Code and Codex reach her: the plugin, or hooks written into
// Claude Code's settings.json; hooks written into Codex's hooks.json (and
// Antigravity's; OpenCode gets a plugin file of ours, omp an extension in
// its agent folder). Read on demand (the person may change any of them at
// any time), and written only when they ask, from the settings.
//
// Each agent's own in a file of its own (claude.rs, codex.rs, agy.rs,
// opencode.rs, omp.rs, zcode.rs, dsh.rs, reasonix.rs), what they share in
// file.rs, their versions in versions.rs; here, what is of them all.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

mod agy;
mod claude;
mod codex;
mod dsh;
mod file;
mod omp;
mod opencode;
mod reasonix;
mod versions;
mod zcode;

pub use agy::*;
pub use claude::*;
pub use codex::*;
pub use dsh::*;
pub use file::*;
pub use omp::*;
pub use opencode::*;
pub use reasonix::*;
pub use versions::*;
pub use zcode::*;

// What kept a settings file from being written, for what to do about it:
// locked (another program has it open), invalid (not JSON), flowList (DSH's
// patch file with its list as [ … ]), path (agy can run no program from
// where she is), pathChars (nor Reasonix's cmd, for a character in that
// path), old (a Reasonix 1.x), other. From
// the error's text: io's ends "(os error 32)" (or 33) whatever the
// language, serde_json's says where, "at line 3 column 5".
pub fn error_kind(error: &str) -> &'static str {
    if error.contains(AGY_NO_NAME) {
        "path"
    } else if error.contains(REASONIX_NO_NAME) {
        "pathChars"
    } else if error.contains(REASONIX_OLD) {
        "old"
    } else if error.contains("(os error 32)") || error.contains("(os error 33)") {
        "locked"
    } else if error.contains(DSH_FLAT) {
        "flowList"
    } else if let Some(rest) = error.strip_prefix(DSH_BRIDGE_FAILED) {
        // The bridge's packages: why, as dsh_bridge.rs tells it.
        match rest.trim_start().strip_prefix('[').and_then(|r| r.split(']').next()).unwrap_or("") {
            "registry" => "bridgeRegistry",
            "version" => "bridgeVersion",
            "notFound" => "bridgeNotFound",
            "releaseAge" => "bridgeReleaseAge",
            "timeout" => "bridgeTimeout",
            "runtime" => "bridgeRuntime",
            "busy" => "bridgeBusy",
            "disk" => "bridgeDisk",
            "permission" => "bridgePermission",
            _ => "bridgeOther",
        }
    } else if error.contains(" at line ") && error.contains(" column ") {
        "invalid"
    } else {
        "other"
    }
}

// Whether any of them is connected, as last looked (at start, as the
// settings read it, after connecting) or heard (any event from them): what
// the island asks for while it is not (island.js), without reading their
// settings files on every redraw. Connected until looked, so a first draw
// never asks wrongly.
static CONNECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn note_connected(connected: bool) -> bool {
    CONNECTED.swap(connected, std::sync::atomic::Ordering::SeqCst) != connected
}

// Whether any of them is, as their files say now: the one list of them, for
// looking (below) and for the settings' snapshot alike (settings.rs; its
// own list there once left ZCode, DeepSeek Harness and Reasonix out, and
// the island asked to connect one of them while it was).
pub fn any_connected(port: u16) -> bool {
    connection(port) != "none" || is_codex_connected() || is_agy_connected() || is_opencode_connected(port) || is_omp_connected(port) || is_zcode_connected() || is_dsh_connected(port) || is_reasonix_connected()
}

pub fn look_connected(port: u16) -> bool {
    let connected = any_connected(port);
    note_connected(connected);
    connected
}

pub fn seems_connected() -> bool {
    CONNECTED.load(std::sync::atomic::Ordering::SeqCst)
}

// The tools on this computer by name, in their order (the island names
// them while none is connected): Claude Code by its folder, Codex,
// Antigravity, OpenCode, omp, ZCode, DeepSeek Harness and Reasonix by
// theirs; none found, all but omp, ZCode, DeepSeek Harness and Reasonix
// (it can not be told).
pub fn tools_here() -> Vec<&'static str> {
    let here: Vec<&str> = [("Claude Code", claude_here()), ("Codex", codex_home().is_dir()), ("Antigravity", agy_here()), ("OpenCode", opencode_here()), ("omp", omp_here()), ("ZCode", zcode_here()), ("DeepSeek Harness", dsh_here()), ("Reasonix", reasonix_here())]
        .into_iter()
        .filter(|(_, is)| *is)
        .map(|(name, _)| name)
        .collect();
    if here.is_empty() {
        vec!["Claude Code", "Codex", "Antigravity", "OpenCode"]
    } else {
        here
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_write_that_failed_says_why() {
        assert_eq!(super::error_kind("另一个程序正在使用此文件，进程无法访问。 (os error 32)"), "locked");
        assert_eq!(super::error_kind("expected `,` or `}` at line 3 column 5"), "invalid");
        assert_eq!(super::error_kind("拒绝访问。 (os error 5)"), "other");
    }
}
