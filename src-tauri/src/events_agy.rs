// Antigravity's hook events, handed to her port by this program with
// connection::AGY_FLAG and the event's name (main.rs agy_hook): agent-core
// reads them (agy.rs to_event, with the person's words and the reply from
// its transcript), events.rs tells them in her words. A PreToolUse that
// Antigravity will ask the person about (needs_approval) comes as Claude
// Code's PermissionRequest (for_ask), answered with auto_approve / deny /
// allow (to_output). Here too, her log's line for a turn.
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[cfg(test)]
use agent_core::agy::with_transcript;

fn s<'a>(e: &'a Value, key: &str) -> &'a str {
    e.get(key).and_then(Value::as_str).unwrap_or("")
}

// Antigravity's user settings (~/.gemini/config/config.json), where its
// permission preset, command policy and "Always Allow" grants live.
pub fn config_file() -> PathBuf {
    crate::connection::gemini_dir().join("config").join("config.json")
}

pub fn read_config() -> Value {
    std::fs::read_to_string(config_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

// A line for her log as a turn begins or ends, or a prompt comes: what
// Antigravity said, to tell a turn that never ended from one shown short.
pub fn log_line(e: &Value) -> Option<String> {
    let name = s(e, "hook_event_name");
    let what = match name {
        "SessionStart" => String::new(),
        "PreInvocation" if e.get("invocationNum").and_then(Value::as_u64) == Some(0) => format!(" 0, the person's step {}", e["userStep"]),
        "PermissionRequest" => format!(" {}", s(e, "tool_name")),
        "Stop" => format!(", fullyIdle {}, {}{}", e["fullyIdle"], s(e, "terminationReason"), if s(e, "error").is_empty() { "" } else { ", an error" }),
        _ => return None,
    };
    Some(format!("agy {name}{what} ({})", s(e, "conversationId").chars().take(8).collect::<String>()))
}

fn norm_path(p: &str) -> String {
    let mut out = p.trim().replace('\\', "/");
    while out.ends_with('/') && out.len() > 1 {
        out.pop();
    }
    if cfg!(windows) {
        out.to_ascii_lowercase()
    } else {
        out
    }
}

fn path_within(path: &str, root: &str) -> bool {
    let (p, r) = (norm_path(path), norm_path(root));
    if p.is_empty() || r.is_empty() {
        return false;
    }
    p == r || p.strip_prefix(&r).is_some_and(|rest| rest.starts_with('/'))
}

fn is_relative_path(path: &str) -> bool {
    let p = path.trim();
    !p.is_empty() && !p.starts_with('/') && !p.starts_with('\\') && p.as_bytes().get(1) != Some(&b':')
}

// Antigravity's isFilePathAllowed allows any path inside workspacePaths or
// its own ~/.gemini/antigravity folder (artifacts, brain, scratch) without
// prompting the person; only paths outside those ask.
fn is_workspace_or_artifact_path(e: &Value, path: &str) -> bool {
    if path.trim().is_empty() || is_relative_path(path) {
        return true;
    }
    if let Some(ws) = e.get("workspacePaths").and_then(Value::as_array) {
        if ws.iter().filter_map(Value::as_str).any(|root| path_within(path, root)) {
            return true;
        }
    }
    let artifact = s(e, "artifactDirectoryPath");
    if !artifact.is_empty() && path_within(path, artifact) {
        return true;
    }
    let agy_dir = crate::connection::gemini_dir().join("antigravity");
    path_within(path, &agy_dir.to_string_lossy())
}

// Parse Antigravity's `<action>(<target>)` grant string.
fn parse_grant(grant: &str) -> Option<(&str, &str)> {
    let g = grant.trim();
    let open = g.find('(')?;
    let rest = g.strip_suffix(')')?;
    let action = &rest[..open];
    let target = &rest[open + 1..];
    (!action.is_empty()).then_some((action, target))
}

// Whether a grant in userSettings.globalPermissionGrants covers (action, target),
// matching language_server's PermissionGrant.Covers / coversCommand / CoversPath / coversMCP.
fn grant_covers(grant: &str, action: &str, target: &str) -> bool {
    let Some((g_action, g_target)) = parse_grant(grant) else { return false };
    if g_action != action {
        return false;
    }
    if g_target == "*" {
        return true;
    }
    match action {
        "command" => {
            let rule_tokens: Vec<&str> = g_target.split_whitespace().collect();
            let cmd_tokens: Vec<&str> = target.split_whitespace().collect();
            !rule_tokens.is_empty() && cmd_tokens.len() >= rule_tokens.len() && cmd_tokens[..rule_tokens.len()] == rule_tokens[..]
        }
        "write_file" | "read_file" => path_within(target, g_target),
        "read_url" => target == g_target || target.starts_with(g_target),
        "mcp" => {
            if let Some(prefix) = g_target.strip_suffix("/*") {
                target.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
            } else {
                target == g_target
            }
        }
        _ => target == g_target,
    }
}

fn first_str<'a>(args: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter().find_map(|k| args.get(*k).and_then(Value::as_str)).unwrap_or("")
}

// Map an Antigravity toolCall to:
// - Claude Code's (tool_name, tool_input) for asks.rs and events.rs
// - Antigravity's permission grant (action, target) when the tool requires
//   permissions in EnsurePermissions (None for internal tools like
//   task_boundary, view_file_outline, ask_question, etc.).
fn classify_tool(raw_tool: &str, raw_args: &Value) -> (String, Value, Option<(&'static str, String)>) {
    match raw_tool {
        "run_command" | "pwsh" | "bash" => {
            let cmd = first_str(raw_args, &["CommandLine", "command"]);
            let desc = first_str(raw_args, &["toolSummary", "toolAction", "description"]);
            let mut input = json!({ "command": cmd });
            if !desc.is_empty() {
                input["description"] = json!(desc);
            }
            let rule = cmd.lines().next().unwrap_or(cmd).trim().to_string();
            ("Bash".into(), input, Some(("command", rule)))
        }
        "send_command_input" => {
            let cmd = first_str(raw_args, &["Input", "CommandId"]);
            let desc = first_str(raw_args, &["toolSummary", "toolAction"]);
            let mut input = json!({ "command": cmd });
            if !desc.is_empty() {
                input["description"] = json!(desc);
            }
            let rule = cmd.lines().next().unwrap_or(cmd).trim().to_string();
            ("Bash".into(), input, Some(("command", rule)))
        }
        "write_file" | "write_to_file" => {
            let path = first_str(raw_args, &["Path", "TargetFile", "file_path"]);
            let content = first_str(raw_args, &["Content", "CodeContent", "content"]);
            ("Write".into(), json!({ "file_path": path, "content": content }), Some(("write_file", path.into())))
        }
        "edit_file" | "replace_file_content" | "multi_replace_file_content" | "sed_file" => {
            let path = first_str(raw_args, &["Path", "TargetFile", "file_path"]);
            let (mut old, mut new) = (
                first_str(raw_args, &["Target", "TargetContent", "old_string"]).to_string(),
                first_str(raw_args, &["Replacement", "ReplacementContent", "new_string"]).to_string(),
            );
            if old.is_empty() && new.is_empty() {
                if let Some(chunk) = raw_args.get("ReplacementChunks").and_then(Value::as_array).and_then(|a| a.first()) {
                    old = first_str(chunk, &["TargetContent"]).to_string();
                    new = first_str(chunk, &["ReplacementContent"]).to_string();
                }
            }
            ("Edit".into(), json!({ "file_path": path, "old_string": old, "new_string": new }), Some(("write_file", path.into())))
        }
        "notebook_edit" => {
            let path = first_str(raw_args, &["NotebookPath", "Path", "TargetFile", "notebook_path"]);
            let src = first_str(raw_args, &["NewSource", "Content", "new_source"]);
            ("NotebookEdit".into(), json!({ "notebook_path": path, "new_source": src }), Some(("write_file", path.into())))
        }
        "read_file" | "view_file" | "list_dir" => {
            let path = first_str(raw_args, &["Path", "AbsolutePath", "DirectoryPath", "file_path"]);
            ("Read".into(), json!({ "file_path": path }), Some(("read_file", path.into())))
        }
        "grep_search" => {
            let path = first_str(raw_args, &["SearchPath", "Path", "path"]);
            let pat = first_str(raw_args, &["Query", "pattern"]);
            ("Grep".into(), json!({ "path": path, "pattern": pat }), Some(("read_file", path.into())))
        }
        "find_by_name" => {
            let path = first_str(raw_args, &["SearchDirectory", "DirectoryPath", "Path", "path"]);
            let pat = first_str(raw_args, &["Pattern", "pattern"]);
            ("Glob".into(), json!({ "path": path, "pattern": pat }), Some(("read_file", path.into())))
        }
        "read_url_content" => {
            let url = first_str(raw_args, &["Url", "url"]);
            ("WebFetch".into(), json!({ "url": url }), Some(("read_url", url.into())))
        }
        "call_mcp_tool" => {
            let server = first_str(raw_args, &["ServerName", "server"]);
            let tool = first_str(raw_args, &["ToolName", "tool"]);
            let args = raw_args.get("Arguments").cloned().unwrap_or_else(|| raw_args.clone());
            let name = if !server.is_empty() && !tool.is_empty() { format!("mcp__{server}__{tool}") } else { raw_tool.into() };
            let target = if !server.is_empty() && !tool.is_empty() { format!("{server}/{tool}") } else { "*".into() };
            (name, args, Some(("mcp", target)))
        }
        other if other.starts_with("mcp_") => {
            let target = if let Some(rest) = other.strip_prefix("mcp__") {
                rest.split_once("__").map_or_else(|| "*".into(), |(srv, tl)| format!("{srv}/{tl}"))
            } else {
                "*".into()
            };
            (other.into(), raw_args.clone(), Some(("mcp", target)))
        }
        other => (other.into(), raw_args.clone(), None),
    }
}

// Whether a PreToolUse event in Antigravity will actually prompt the person in
// EnsurePermissions: internal tools, workspace/artifact file edits, grants
// already in globalPermissionGrants.allow, and Turbo / Eager auto-execution
// modes run without asking, so the hook answers {"decision":"allow"} at once.
pub fn needs_approval(e: &Value, config: &Value) -> bool {
    let call = e.get("toolCall").unwrap_or(&Value::Null);
    let raw_tool = s(call, "name");
    let raw_args = call.get("args").unwrap_or(&Value::Null);
    let (_, _, Some((action, target))) = classify_tool(raw_tool, raw_args) else {
        return false;
    };
    let settings = config.get("userSettings").unwrap_or(&Value::Null);
    let grants = settings.get("globalPermissionGrants").unwrap_or(&Value::Null);
    let matches_any = |key: &str| {
        grants
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().filter_map(Value::as_str).any(|g| grant_covers(g, action, &target)))
    };
    if matches_any("ask") {
        return true;
    }
    if matches_any("allow") || matches_any("deny") {
        return false;
    }
    if matches!(action, "write_file" | "read_file") && is_workspace_or_artifact_path(e, &target) {
        return false;
    }
    if s(settings, "permissionPreset") == "AGENT_PERMISSION_PRESET_TURBO" {
        return false;
    }
    match action {
        "command" => {
            let policy = s(settings, "autoExecutionPolicy");
            if policy == "CASCADE_COMMANDS_AUTO_EXECUTION_EAGER" {
                return false;
            }
            if policy == "CASCADE_COMMANDS_AUTO_EXECUTION_PROCEED_IN_SANDBOX"
                && settings.get("enableTerminalSandbox").and_then(Value::as_bool) == Some(true)
            {
                return false;
            }
            true
        }
        "write_file" | "read_file" => s(settings, "nonWorkspaceFileAccessPolicy") != "AGENT_SETTING_POLICY_ALLOW",
        "read_url" | "mcp" => true,
        _ => false,
    }
}

// A tool call needing approval in Antigravity (PreToolUse): worded for asks.rs
// as Claude Code's PermissionRequest so she shows it on her island/panel.
pub fn for_ask(e: &Value) -> Value {
    let call = e.get("toolCall").cloned().unwrap_or_else(|| json!({}));
    let raw_tool = call.get("name").and_then(Value::as_str).unwrap_or("");
    let raw_args = call.get("args").cloned().unwrap_or_else(|| json!({}));
    let (tool, input, grant) = classify_tool(raw_tool, &raw_args);
    let session = s(e, "conversationId");
    let cwd = e.get("workspacePaths").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).unwrap_or("");
    let step = e.get("stepIdx").and_then(Value::as_i64).map(|v| v.to_string()).unwrap_or_default();
    let mut out = json!({
        "hook_event_name": "PermissionRequest",
        "session_id": session,
        "conversationId": session,
        "cwd": cwd,
        "agent": "agy",
        "tool_name": tool,
        "tool_input": input,
        "tool_use_id": format!("{session}-{step}"),
    });
    if let Some((action, target)) = grant {
        if !target.is_empty() {
            out["permission_suggestions"] = json!([{
                "type": "addRules",
                "behavior": "allow",
                "rules": [{ "toolName": action, "ruleContent": target }],
                "destination": "userSettings"
            }]);
        }
    }
    out
}

// Add session_id (and on PostToolUse the normalized tool_use_id, tool_name and
// tool_input) to an Antigravity event so asks.seen() settles any prompt
// answered in Antigravity's client or ended by Stop.
pub fn with_seen(e: &mut Value) {
    let session = s(e, "conversationId").to_string();
    if session.is_empty() {
        return;
    }
    let is_post = s(e, "hook_event_name") == "PostToolUse";
    let step = e.get("stepIdx").and_then(Value::as_i64).map(|v| v.to_string());
    let mapped = is_post.then(|| {
        let call = e.get("toolCall").unwrap_or(&Value::Null);
        classify_tool(s(call, "name"), call.get("args").unwrap_or(&Value::Null))
    });
    if let Some(fields) = e.as_object_mut() {
        fields.entry("session_id".to_string()).or_insert_with(|| json!(session));
        if let Some((tool, input, _)) = mapped {
            fields.entry("tool_name".to_string()).or_insert_with(|| json!(tool));
            fields.entry("tool_input".to_string()).or_insert(input);
            if let Some(step) = step {
                fields.entry("tool_use_id".to_string()).or_insert_with(|| json!(format!("{session}-{step}")));
            }
        }
    }
}

// Persist "Always Allow" grants into ~/.gemini/config/config.json
// (userSettings.globalPermissionGrants.allow), which language_server watches.
pub fn save_always_grants_to(file: &Path, rules: &[String]) {
    if rules.is_empty() {
        return;
    }
    let mut root: Value = if file.exists() {
        let Ok(text) = std::fs::read_to_string(file) else { return };
        let Ok(val) = serde_json::from_str(&text) else { return };
        val
    } else {
        json!({})
    };
    let Some(root_obj) = root.as_object_mut() else { return };
    let settings = root_obj.entry("userSettings").or_insert_with(|| json!({}));
    let Some(settings_obj) = settings.as_object_mut() else { return };
    let grants = settings_obj.entry("globalPermissionGrants").or_insert_with(|| json!({}));
    let Some(grants_obj) = grants.as_object_mut() else { return };
    let allow = grants_obj.entry("allow").or_insert_with(|| json!([]));
    let Some(arr) = allow.as_array_mut() else { return };
    let mut changed = false;
    for rule in rules {
        if parse_grant(rule).is_some() && !arr.iter().any(|v| v.as_str() == Some(rule.as_str())) {
            arr.push(json!(rule));
            changed = true;
        }
    }
    if changed {
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&root) {
            let _ = std::fs::write(file, format!("{text}\n"));
        }
    }
}

fn extract_rules(perms: &[Value]) -> Vec<String> {
    let mut rules = Vec::new();
    for p in perms {
        if let Some(arr) = p.get("rules").and_then(Value::as_array) {
            for r in arr {
                if let Some(s) = r.as_str() {
                    if !s.is_empty() {
                        rules.push(s.to_string());
                    }
                } else if r.is_object() {
                    let tool = s(r, "toolName");
                    let content = s(r, "ruleContent");
                    if !tool.is_empty() {
                        rules.push(if content.is_empty() { tool.to_string() } else { format!("{tool}({content})") });
                    }
                }
            }
        }
    }
    rules
}

// What to return to Antigravity on stdout for her decision:
// - Allow once -> {"decision":"auto_approve"} (bypasses Antigravity's native dialog)
// - Always allow -> {"decision":"auto_approve","permissionOverrides":[...]} + saved to config.json
// - Deny -> {"decision":"deny","reason":"..."}
// - Dismiss / timeout / out of sight -> {"decision":"allow"} (defers to Antigravity's native dialog)
pub fn to_output(body: &str) -> String {
    #[cfg(not(test))]
    let persist = Some(config_file());
    #[cfg(test)]
    let persist: Option<PathBuf> = None;
    to_output_with(body, persist.as_deref())
}

pub fn to_output_with(body: &str, config: Option<&Path>) -> String {
    let Ok(val) = serde_json::from_str::<Value>(body) else {
        return r#"{"decision":"allow"}"#.to_string();
    };
    let Some(decision) = val.get("hookSpecificOutput").and_then(|h| h.get("decision")) else {
        return r#"{"decision":"allow"}"#.to_string();
    };
    match decision.get("behavior").and_then(Value::as_str) {
        Some("allow") => {
            if let Some(perms) = decision.get("updatedPermissions").and_then(Value::as_array) {
                let rules = extract_rules(perms);
                if !rules.is_empty() {
                    if let Some(file) = config {
                        save_always_grants_to(file, &rules);
                    }
                    return json!({ "decision": "auto_approve", "permissionOverrides": rules }).to_string();
                }
            }
            r#"{"decision":"auto_approve"}"#.to_string()
        }
        Some("deny") => {
            let msg = decision.get("message").and_then(Value::as_str).unwrap_or("用户在灵动岛上拒绝了此操作");
            json!({ "decision": "deny", "reason": msg }).to_string()
        }
        _ => r#"{"decision":"allow"}"#.to_string(),
    }
}

#[cfg(test)]
fn to_message(e: &Value) -> Option<Value> {
    let mut e = e.clone();
    agent_core::agy::to_event(&mut e).map(|ev| crate::events::message_of(&ev))
}
#[cfg(test)]
mod tests {
    use super::*;

    // As agy 1.3.4 sends them (trimmed), the event's name added by main.rs.
    fn event(name: &str, more: Value) -> Value {
        let mut e = json!({
            "hook_event_name": name,
            "conversationId": "5064aa2e",
            "modelName": "gemini-3.8-flash-high",
            "workspacePaths": ["Z:/nowhere/agyws"],
            "transcriptPath": "",
        });
        for (k, v) in more.as_object().unwrap() {
            e[k] = v.clone();
        }
        e
    }

    #[test]
    fn agy_events_become_messages() {
        let m = to_message(&event("SessionStart", json!({}))).unwrap();
        assert_eq!(m, json!({ "mood": "idle", "react": "wave", "say": { "key": "say.hello" }, "agent": "agy", "session": "5064aa2e", "project": "agyws" }));
        let m = to_message(&event("PreInvocation", json!({ "invocationNum": 0, "initialNumSteps": 1, "prompt": "Create a file named hello.txt" }))).unwrap();
        assert_eq!((m["event"].as_str(), m["task"].as_str()), (Some("turn-start"), Some("Create a file named hello.txt")));
        let m = to_message(&event("PreInvocation", json!({ "invocationNum": 1, "initialNumSteps": 3 }))).unwrap();
        assert_eq!((m["mood"].as_str(), m.get("event")), (Some("working"), None));
        let call = json!({ "name": "write_to_file", "args": { "CodeContent": "hi", "TargetFile": "Z:\\nowhere\\agyws\\hello.txt", "toolAction": "Creating hello.txt", "toolSummary": "Create hello.txt" } });
        let mut done = event("PostToolUse", json!({ "stepIdx": 2, "error": "", "toolCall": call }));
        let m = to_message(&done).unwrap();
        assert_eq!((m["event"].as_str(), m["file"].as_str()), (Some("tool-done"), Some("Z:\\nowhere\\agyws\\hello.txt")));
        // Its own words for what it did go with the call.
        match agent_core::agy::to_event(&mut done).unwrap().kind {
            agent_core::event::Kind::ToolDone { call } => assert_eq!(call.intent.as_deref(), Some("Creating hello.txt")),
            other => panic!("{other:?}"),
        }
        let m = to_message(&event("PostToolUse", json!({ "error": "exit status 1", "toolCall": { "name": "run_command", "args": {} } }))).unwrap();
        assert_eq!((m["event"].as_str(), m["react"].as_str()), (Some("tool-failed"), Some("failed")));
        let m = to_message(&event("Stop", json!({ "executionNum": 0, "fullyIdle": true, "terminationReason": "NO_TOOL_CALL", "error": "", "last_assistant_message": "done" }))).unwrap();
        assert_eq!((m["mood"].as_str(), m["event"].as_str(), m["reply"].as_str()), (Some("done"), Some("turn-end"), Some("done")));
        let m = to_message(&event("Stop", json!({ "fullyIdle": true, "error": "quota exceeded" }))).unwrap();
        assert_eq!((m["mood"].as_str(), m["error"].as_str()), (Some("error"), Some("quota exceeded")));
        let m = to_message(&event("Stop", json!({ "fullyIdle": true, "terminationReason": "USER_CANCELLED", "error": "" }))).unwrap();
        assert_eq!((m["mood"].as_str(), m["event"].as_str()), (Some("idle"), Some("turn-end")));
    }

    #[test]
    fn agy_events_that_change_nothing() {
        assert!(to_message(&event("Stop", json!({ "fullyIdle": false, "error": "" }))).is_none());
        for name in ["PostInvocation", "PreToolUse", "Nope", ""] {
            assert!(to_message(&event(name, json!({}))).is_none(), "{name}");
        }
        assert!(to_message(&json!("nope")).is_none());
    }

    #[test]
    fn the_person_s_words_and_the_reply_from_the_transcript() {
        let file = std::env::temp_dir().join(format!("wakuwaku-agy-{}.jsonl", std::process::id()));
        let lines = [
            json!({ "step_index": 0, "source": "USER_EXPLICIT", "type": "USER_INPUT", "status": "DONE", "content": "<USER_REQUEST>\nCreate a file named hello.txt\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nThe current local time is 09:47.\n</ADDITIONAL_METADATA>" }),
            json!({ "step_index": 1, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "thinking": "write it", "tool_calls": [] }),
            json!({ "step_index": 3, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "content": "done" }),
        ];
        std::fs::write(&file, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let path = file.to_string_lossy().into_owned();
        let mut e = event("PreInvocation", json!({ "invocationNum": 0, "transcriptPath": path }));
        with_transcript(&mut e);
        assert_eq!((e["prompt"].as_str(), e["userStep"].as_u64()), (Some("Create a file named hello.txt"), Some(0)));
        assert_eq!(to_message(&e).unwrap()["turn"], "0");
        let mut e = event("Stop", json!({ "fullyIdle": true, "transcriptPath": path }));
        with_transcript(&mut e);
        assert_eq!(e["last_assistant_message"], "done");
        // A later call of the model reads nothing.
        let mut e = event("PreInvocation", json!({ "invocationNum": 1, "transcriptPath": path }));
        with_transcript(&mut e);
        assert!(e.get("prompt").is_none());
        // The person's next words, an image alone: a turn of its own, without words.
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        std::io::Write::write_all(&mut f, format!("\n{}", json!({ "step_index": 4, "type": "USER_INPUT", "content": "" })).as_bytes()).unwrap();
        let mut e = event("PreInvocation", json!({ "invocationNum": 0, "transcriptPath": path }));
        with_transcript(&mut e);
        assert_eq!((e.get("prompt"), e["userStep"].as_u64()), (None, Some(4)));
        let _ = std::fs::remove_file(&file);
    }

    // Each event as the server takes it, applied at a time (ms): the turns it
    // began and the time it counted.
    fn feed(pet: &mut crate::state::Pet, name: &str, more: Value, at: u64) -> (u64, u64) {
        let mut e = event(name, more);
        with_transcript(&mut e);
        agent_core::agy::to_event(&mut e).and_then(|ev| pet.apply(&ev, None, at, crate::state::Hold::Seen)).map_or((0, 0), |out| (out.turns, out.worked_ms))
    }

    #[test]
    fn an_agy_round_counts_its_turn_and_its_time() {
        // The plainest: the model called, a tool, the end.
        let mut pet = crate::state::Pet::default();
        assert_eq!(feed(&mut pet, "SessionStart", json!({}), 0), (0, 0));
        assert_eq!(feed(&mut pet, "PreInvocation", json!({ "invocationNum": 0 }), 1_000), (1, 0));
        let call = json!({ "name": "write_to_file", "args": { "TargetFile": "Z:\\nowhere\\agyws\\a.txt", "toolAction": "Creating a.txt" } });
        assert_eq!(feed(&mut pet, "PostToolUse", json!({ "error": "", "toolCall": call }), 4_000), (0, 0));
        assert_eq!(feed(&mut pet, "Stop", json!({ "fullyIdle": true, "terminationReason": "NO_TOOL_CALL", "error": "" }), 6_500), (0, 5_500));

        // A command left running in the background, as agy 1.3.4 ran one: the
        // model answers (not fully idle), the command ends, the model is
        // called again from 0, then the end. One turn, all of its time.
        let file = std::env::temp_dir().join(format!("wakuwaku-agy-round-{}.jsonl", std::process::id()));
        let path = file.to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&file);
        let write = |steps: &[Value]| {
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&file).unwrap();
            let lines: String = steps.iter().map(|s| format!("{s}\n")).collect();
            std::io::Write::write_all(&mut f, lines.as_bytes()).unwrap();
        };
        let at = |more: Value| {
            let mut more = more;
            more["transcriptPath"] = json!(path);
            more
        };
        let mut pet = crate::state::Pet::default();
        write(&[json!({ "step_index": 0, "source": "USER_EXPLICIT", "type": "USER_INPUT", "status": "DONE", "content": "<USER_REQUEST>\nping in the background\n</USER_REQUEST>" })]);
        assert_eq!(feed(&mut pet, "PreInvocation", at(json!({ "invocationNum": 0, "initialNumSteps": 1 })), 100), (1, 0));
        write(&[
            json!({ "step_index": 1, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "tool_calls": [{ "name": "run_command" }] }),
            json!({ "step_index": 2, "source": "MODEL", "type": "GENERIC", "status": "RUNNING", "content": "Tool is running as a background task" }),
        ]);
        assert_eq!(feed(&mut pet, "PreInvocation", at(json!({ "invocationNum": 1, "initialNumSteps": 3 })), 9_000), (0, 0));
        write(&[json!({ "step_index": 3, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "content": "started" })]);
        assert_eq!(feed(&mut pet, "Stop", at(json!({ "executionNum": 0, "fullyIdle": false, "terminationReason": "NO_TOOL_CALL", "error": "" })), 13_800), (0, 0));
        assert_eq!(pet.get(13_900)["mood"], "working");
        assert_eq!(feed(&mut pet, "PostToolUse", at(json!({ "stepIdx": 2, "error": "", "toolCall": { "name": "run_command", "args": { "toolAction": "Running ping" } } })), 20_300), (0, 0));
        write(&[json!({ "step_index": 4, "source": "SYSTEM", "type": "SYSTEM_MESSAGE", "status": "DONE", "content": "the task is done" })]);
        assert_eq!(feed(&mut pet, "PreInvocation", at(json!({ "invocationNum": 0, "initialNumSteps": 4 })), 20_500), (0, 0));
        write(&[json!({ "step_index": 5, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "content": "It finished." })]);
        assert_eq!(feed(&mut pet, "Stop", at(json!({ "executionNum": 0, "fullyIdle": true, "terminationReason": "NO_TOOL_CALL", "error": "" })), 23_600), (0, 23_500));
        assert_eq!((pet.get(23_700)["mood"].as_str(), pet.get(23_700)["reply"].as_str()), (Some("done"), Some("It finished.")));

        // The person's next words, stopped by them: a turn, and its time.
        write(&[json!({ "step_index": 6, "source": "USER_EXPLICIT", "type": "USER_INPUT", "status": "DONE", "content": "<USER_REQUEST>\nnext\n</USER_REQUEST>" })]);
        assert_eq!(feed(&mut pet, "PreInvocation", at(json!({ "invocationNum": 0, "initialNumSteps": 7 })), 60_000), (1, 0));
        assert_eq!(feed(&mut pet, "Stop", at(json!({ "fullyIdle": true, "terminationReason": "USER_CANCELED", "error": "" })), 70_000), (0, 10_000));
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn turns_begin_and_end_in_her_log() {
        let line = |name: &str, more: Value| log_line(&event(name, more));
        assert_eq!(line("SessionStart", json!({})).as_deref(), Some("agy SessionStart (5064aa2e)"));
        assert_eq!(line("PreInvocation", json!({ "invocationNum": 0, "userStep": 6 })).as_deref(), Some("agy PreInvocation 0, the person's step 6 (5064aa2e)"));
        assert_eq!(line("PermissionRequest", json!({ "tool_name": "Bash" })).as_deref(), Some("agy PermissionRequest Bash (5064aa2e)"));
        assert_eq!(line("Stop", json!({ "fullyIdle": false, "terminationReason": "NO_TOOL_CALL", "error": "" })).as_deref(), Some("agy Stop, fullyIdle false, NO_TOOL_CALL (5064aa2e)"));
        assert!(line("PreInvocation", json!({ "invocationNum": 1 })).is_none() && line("PostToolUse", json!({})).is_none());
    }

    #[test]
    fn agy_pre_tool_use_becomes_ask_and_replies() {
        let call = json!({ "name": "run_command", "args": { "CommandLine": "npm test", "toolSummary": "Run tests" } });
        let pre = json!({ "hook_event_name": "PreToolUse", "conversationId": "5064aa2e", "workspacePaths": ["Z:/nowhere/agyws"], "toolCall": call, "stepIdx": 5 });
        let ask = for_ask(&pre);
        assert_eq!((ask["hook_event_name"].as_str(), ask["tool_name"].as_str()), (Some("PermissionRequest"), Some("Bash")));
        assert_eq!(ask["tool_input"]["command"], "npm test");
        assert_eq!(ask["tool_input"]["description"], "Run tests");
        assert_eq!(ask["permission_suggestions"][0]["rules"][0], json!({ "toolName": "command", "ruleContent": "npm test" }));
        assert_eq!(ask["permission_suggestions"][0]["destination"], "userSettings");

        // asks.rs view_of and reply_for round-trip
        let view = agent_core::asks::view_of(1, &ask);
        assert_eq!(view["always"]["rules"], json!(["command(npm test)"]));
        assert_eq!(view["always"]["where"], json!(["userSettings"]));

        // Allow once -> auto_approve (bypasses Antigravity's native dialog)
        let allow = agent_core::asks::reply_for(&ask, &json!({ "action": "allow" }), "denied").unwrap();
        assert_eq!(to_output(&allow.to_string()), r#"{"decision":"auto_approve"}"#);

        // Always allow -> auto_approve + permissionOverrides + persisted to config.json
        let cfg_file = std::env::temp_dir().join(format!("wakuwaku-agy-cfg-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&cfg_file);
        let always = agent_core::asks::reply_for(&ask, &json!({ "action": "always" }), "denied").unwrap();
        assert_eq!(
            to_output_with(&always.to_string(), Some(&cfg_file)),
            r#"{"decision":"auto_approve","permissionOverrides":["command(npm test)"]}"#
        );
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&cfg_file).unwrap()).unwrap();
        assert_eq!(saved["userSettings"]["globalPermissionGrants"]["allow"], json!(["command(npm test)"]));

        // Once saved in config.json, needs_approval is false for `npm test` and `npm test --watch`
        assert!(!needs_approval(&pre, &saved));
        let pre_watch = json!({ "hook_event_name": "PreToolUse", "conversationId": "5064aa2e", "workspacePaths": ["Z:/nowhere/agyws"], "toolCall": { "name": "run_command", "args": { "CommandLine": "npm test --watch" } } });
        assert!(!needs_approval(&pre_watch, &saved));
        let pre_other = json!({ "hook_event_name": "PreToolUse", "conversationId": "5064aa2e", "workspacePaths": ["Z:/nowhere/agyws"], "toolCall": { "name": "run_command", "args": { "CommandLine": "cargo build" } } });
        assert!(needs_approval(&pre_other, &saved));
        let _ = std::fs::remove_file(&cfg_file);

        // Deny -> deny + reason
        let deny = agent_core::asks::reply_for(&ask, &json!({ "action": "deny", "message": "no" }), "denied").unwrap();
        assert_eq!(to_output(&deny.to_string()), r#"{"decision":"deny","reason":"no"}"#);

        // Dismiss / Timeout / Out of sight -> allow (defers to Antigravity's native dialog)
        assert_eq!(to_output("{}"), r#"{"decision":"allow"}"#);
    }

    #[test]
    fn agy_needs_approval_filters_auto_allowed_tools_and_modes() {
        let default_cfg = json!({});
        // Command in default mode needs approval
        let cmd = event("PreToolUse", json!({ "toolCall": { "name": "run_command", "args": { "CommandLine": "git status" } } }));
        assert!(needs_approval(&cmd, &default_cfg));

        // Turbo preset or Eager autoExecutionPolicy auto-allows commands
        let turbo_cfg = json!({ "userSettings": { "permissionPreset": "AGENT_PERMISSION_PRESET_TURBO" } });
        assert!(!needs_approval(&cmd, &turbo_cfg));
        let eager_cfg = json!({ "userSettings": { "autoExecutionPolicy": "CASCADE_COMMANDS_AUTO_EXECUTION_EAGER" } });
        assert!(!needs_approval(&cmd, &eager_cfg));

        // File edit inside workspacePaths is auto-allowed; outside workspacePaths needs approval
        let in_ws = event("PreToolUse", json!({ "toolCall": { "name": "write_to_file", "args": { "TargetFile": "Z:\\nowhere\\agyws\\src\\lib.rs" } } }));
        assert!(!needs_approval(&in_ws, &default_cfg));
        let out_ws = event("PreToolUse", json!({ "toolCall": { "name": "write_to_file", "args": { "TargetFile": "C:\\outside\\secret.txt" } } }));
        assert!(needs_approval(&out_ws, &default_cfg));
        let allow_non_ws = json!({ "userSettings": { "nonWorkspaceFileAccessPolicy": "AGENT_SETTING_POLICY_ALLOW" } });
        assert!(!needs_approval(&out_ws, &allow_non_ws));

        // Internal tools (ask_question, task_boundary, view_file_outline) never need EnsurePermissions approval
        for tool in ["ask_question", "task_boundary", "view_file_outline", "search_web"] {
            let internal = event("PreToolUse", json!({ "toolCall": { "name": tool, "args": {} } }));
            assert!(!needs_approval(&internal, &default_cfg), "{tool}");
        }
    }

    #[test]
    fn agy_with_seen_settles_pending_prompts_on_post_tool_use_and_stop() {
        let pre = event("PreToolUse", json!({ "stepIdx": 5, "toolCall": { "name": "run_command", "args": { "CommandLine": "npm test" } } }));
        let ask = for_ask(&pre);
        let mut asks: agent_core::asks::Asks<Box<dyn FnOnce(Option<Value>) + Send>> = agent_core::asks::Asks::default();
        let id = asks.add(ask, Box::new(|_| {}), 1_000, 60_000).unwrap();

        let mut post = event("PostToolUse", json!({ "stepIdx": 5, "error": "", "toolCall": { "name": "run_command", "args": { "CommandLine": "npm test" } } }));
        with_seen(&mut post);
        assert_eq!(asks.seen(&post), vec![id]);
        assert!(asks.is_empty());
    }
}
