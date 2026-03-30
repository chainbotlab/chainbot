//! [INPUT]
//! CLI help topics, version metadata, and stable config examples for the ChainBot command surface.
//!
//! [OUTPUT]
//! Renders user-facing help text and per-topic guidance for CLI commands and workspace configuration.
//!
//! [ROLE]
//! Centralizes the canonical help content served by the application CLI layer.

use super::HelpTopic;

const CHAINBOT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GENERAL_HELP_EXAMPLE_HINT: &str =
    "Use `chainbot help validate` for end-to-end config examples.";
const ROOT_CONFIG_EXAMPLE: &str = r#"# chainbot.toml
manifest_version = "2.0.0"
chainbot_version = "2.2.0"
profile = "basic"
secret_refs = ["secret://ops/slack/webhook#token"]

[paths]
workflows_dir = "workflows"
triggers_dir = "triggers"
plugins_dir = "plugins"
secrets_dir = "secrets"
state_dir = "state"

[runtime_defaults]
timezone = "UTC"
"#;
const WORKFLOW_CONFIG_EXAMPLE: &str = r#"# workflows/wf-alpha/config.toml
[workflow]
manifest_version = "2.0.0"
id = "wf-alpha"
name = "alpha"
description = "Normalize a quote payload"

[runtime.defaults]
symbol = "BTCUSDT"

[[nodes]]
manifest_version = "2.0.0"
id = "normalize"
kind = "plugin"
plugin = "quote-plugin"
operation = "normalize"
depends_on = []
"#;
const TRIGGER_CONFIG_EXAMPLE: &str = r#"# triggers/tr-market/config.toml
manifest_version = "2.0.0"
trigger_id = "tr-market"
kind = "builtin"
source = "market_tick"
workflow_id = "wf-alpha"
enabled = true

[params]
symbol = "BTCUSDT"

[input_mapping]
symbol = "payload.symbol"
price = "payload.price"
"#;
const WEBHOOK_TRIGGER_CONFIG_EXAMPLE: &str = r#"# triggers/tr-webhook/config.toml
manifest_version = "2.0.0"
trigger_id = "tr-webhook"
kind = "builtin"
source = "webhook"
workflow_id = "wf-alpha"
enabled = true

[params]
bind = "127.0.0.1:8080"
path = "/ingress/webhook"
method = "POST"
max_body_bytes = 65536
content_type = "application/json"
idempotency_header = "x-event-id"

[params.auth]
kind = "header_token"
header_name = "x-chainbot-token"
token = "dev-webhook-token"

[input_mapping]
symbol = "payload.symbol"
price = "payload.price"
event_id = "payload.id"
"#;
const WEBSOCKET_TRIGGER_CONFIG_EXAMPLE: &str = r#"# triggers/tr-websocket/config.toml
manifest_version = "2.0.0"
trigger_id = "tr-websocket"
kind = "builtin"
source = "websocket"
workflow_id = "wf-alpha"
enabled = true

[params]
bind = "127.0.0.1:8081"
path = "/ingress/ws"
max_connections = 32
max_message_bytes = 65536
idle_timeout_ms = 30000

[params.auth]
kind = "header_token"
header_name = "x-chainbot-token"
token = "dev-websocket-token"

[input_mapping]
symbol = "payload.symbol"
price = "payload.price"
event = "payload.event"
"#;
const PLUGIN_CONFIG_EXAMPLE: &str = r#"# plugins/quote-plugin/config.toml
manifest_version = "2.0.0"
plugin_id = "quote-plugin"
kind = "external_node"
entrypoint = "node.exec.v1"
capabilities = ["node:execute"]
executable = "bin/quote-plugin.sh"

[[operations]]
name = "normalize"
summary = "Normalize quote payload"
input_schema = ["symbol", "token"]
output_schema = ["decision"]
"#;

fn push_help_list_section(lines: &mut Vec<String>, title: &str, items: &[&str]) {
    if items.is_empty() {
        return;
    }

    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!("{title}:"));
    for item in items {
        lines.push(format!("  - {item}"));
    }
}

fn push_help_code_block(lines: &mut Vec<String>, title: &str, language: &str, body: &str) {
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!("{title}:"));
    lines.push(format!("```{language}"));
    lines.extend(body.lines().map(str::to_owned));
    lines.push(String::from("```"));
}

fn render_help_card(
    name: &str,
    summary: &str,
    usage: &[&str],
    use_when: &[&str],
    reads: &[&str],
    writes: &[&str],
    does_not_execute: &[&str],
    outputs: &[&str],
    root_resolution: &[&str],
    config_examples: &[(&str, &str, &str)],
    failure_navigation: &[&str],
    examples: &[&str],
    see_also: &[&str],
) -> String {
    let mut lines = vec![format!("{name} - {summary}")];
    push_help_list_section(&mut lines, "Usage", usage);
    push_help_list_section(&mut lines, "Use when", use_when);
    push_help_list_section(&mut lines, "Reads", reads);
    push_help_list_section(&mut lines, "Writes", writes);
    push_help_list_section(&mut lines, "Does not execute", does_not_execute);
    push_help_list_section(&mut lines, "Outputs", outputs);
    push_help_list_section(&mut lines, "Root resolution", root_resolution);
    for (title, language, body) in config_examples {
        push_help_code_block(&mut lines, title, language, body);
    }
    push_help_list_section(&mut lines, "Failure navigation", failure_navigation);
    push_help_list_section(&mut lines, "Examples", examples);
    push_help_list_section(&mut lines, "See also", see_also);
    lines.join("\n")
}

pub(super) fn general_help_text() -> String {
    let version_line = format!("chainbot {CHAINBOT_VERSION}");
    let mut lines = vec![String::from("ChainBot command skills")];
    push_help_list_section(&mut lines, "Version", &[version_line.as_str()]);
    push_help_list_section(
        &mut lines,
        "Usage",
        &[
            "chainbot help [command]",
            "chainbot version",
            "chainbot init",
            "chainbot status [--json]",
            "chainbot observe [--json] [--limit <n>] [--trigger-id <id>] [--run-id <id>]",
            "chainbot catalog list [--json] [--kind <builtin_node|builtin_trigger|plugin>]",
            "chainbot catalog show <reference> [--json]",
            "chainbot plugin <source|install> ...",
            "chainbot stop",
            "chainbot trigger list [--json]",
            "chainbot trigger <enable|disable> <trigger-id>",
            "chainbot validate",
            "chainbot list-runs",
            "chainbot run",
            "chainbot serve",
        ],
    );
    push_help_list_section(
        &mut lines,
        "Root resolution",
        &[
            "use CHAINBOT_CONFIG_DIR when it is set to a non-empty path",
            "otherwise fall back to ~/.chainbot",
            "root config must be <root>/chainbot.toml",
        ],
    );
    push_help_list_section(
        &mut lines,
        "Command catalog",
        &[
            "version    Print the running ChainBot version.",
            "init       Bootstrap a minimal ChainBot root.",
            "status     Inspect runtime state without executing workflows.",
            "observe    Inspect persisted trigger events, workflow logs, and runs.",
            "catalog    Discover builtin capabilities and installed plugin contracts.",
            "plugin     Discover and install remote plugins from github/git sources.",
            "stop       Request graceful daemon shutdown.",
            "trigger    Inspect or persist trigger package state.",
            "validate   Validate config and package contracts.",
            "list-runs  Print persisted run summaries as JSON.",
            "run        Execute one single-shot manual run.",
            "serve      Start the background daemon control plane.",
        ],
    );
    push_help_list_section(
        &mut lines,
        "AI workflow hints",
        &[
            "start with `chainbot help <command>` before generating automation around a command",
            GENERAL_HELP_EXAMPLE_HINT,
            "prefer `chainbot status --json` and `chainbot trigger list --json` for machine-readable snapshots",
            "use `chainbot catalog list --json` when an agent needs a capability inventory",
            "use `chainbot catalog show <reference> --json` for one capability contract",
            "use `chainbot plugin source list --json` to inspect remote installable plugins before writing to the current root",
            "use `chainbot observe --json` when an agent needs recent persisted events or logs",
        ],
    );
    lines.join("\n")
}

pub(super) fn help_text(topic: HelpTopic) -> String {
    match topic {
        HelpTopic::General => general_help_text(),
        HelpTopic::Version => render_help_card(
            "version",
            "Print the running ChainBot version",
            &["chainbot version", "chainbot --version"],
            &[
                "you need to confirm the installed CLI release",
                "you want to compare the binary version against root config metadata",
            ],
            &[],
            &[],
            &[],
            &["prints `chainbot <version>`"],
            &[],
            &[],
            &["if the reported version mismatches your root metadata, run `chainbot validate` next"],
            &["chainbot version", "chainbot --version"],
            &["init", "validate"],
        ),
        HelpTopic::Init => render_help_card(
            "init",
            "Bootstrap a minimal ChainBot root",
            &["chainbot init"],
            &[
                "you need a new ChainBot root that validates immediately",
                "you want the canonical single-file root config without manual setup",
                "you are preparing a fresh local root for workflows, triggers, plugins, secrets, and state",
            ],
            &[],
            &[
                "resolved root directory",
                "<root>/chainbot.toml when no root config exists yet",
                "default package directories under the resolved root",
            ],
            &["workflow runs", "trigger snapshots"],
            &[
                "prints created and reused bootstrap paths",
                "reuses existing canonical files and directories instead of overwriting them",
            ],
            &[
                "uses CHAINBOT_CONFIG_DIR when it is set to a non-empty path",
                "otherwise bootstraps ~/.chainbot",
            ],
            &[("Root config example", "toml", ROOT_CONFIG_EXAMPLE)],
            &[
                "directory/file collisions are reported with the exact path that blocks bootstrap",
                "invalid existing chainbot.toml is reported with file and line context",
            ],
            &["chainbot init", "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot init"],
            &["validate", "status", "trigger"],
        ),
        HelpTopic::Status => render_help_card(
            "status",
            "Inspect runtime state without executing workflows",
            &["chainbot status", "chainbot status --json"],
            &[
                "you want to know whether serve is active",
                "you want the latest workflow run result without opening state files manually",
                "you want trigger activity and summary counts in one snapshot",
            ],
            &[
                "configured root config",
                "configured workflow packages",
                "configured trigger packages",
                "configured state runs directory",
                "configured trigger record directory",
                "configured coordination store",
            ],
            &[],
            &["workflow runs", "trigger snapshots", "runtime recovery"],
            &[
                "prints a human-readable Root / Workflows / Triggers / Summary snapshot by default",
                "prints stable JSON when `--json` is enabled",
            ],
            &[
                "resolve the root from CHAINBOT_CONFIG_DIR before reading workspace state",
                "return validation errors instead of partial snapshots when config is invalid",
            ],
            &[],
            &[
                "unexpected flags are reported with their argument position after `chainbot status`",
                "invalid root config is reported with file and line context before any runtime access",
            ],
            &[
                "chainbot status",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot status",
                "chainbot status --json",
            ],
            &["validate", "list-runs", "serve"],
        ),
        HelpTopic::Observe => render_help_card(
            "observe",
            "Inspect persisted trigger events, workflow logs, and run history",
            &[
                "chainbot observe",
                "chainbot observe --json",
                "chainbot observe --limit 20 --trigger-id tr-market",
                "chainbot observe --run-id manual-wf-alpha-1710000000000",
            ],
            &[
                "you need the recent persisted trigger-event stream without opening the database manually",
                "you want workflow log lines and run summaries that agree with runtime history",
                "you need to see whether older data has already moved into archive tables",
            ],
            &[
                "persisted run summaries",
                "persisted workflow runtime logs",
                "persisted trigger event records",
                "archive table counts when retention is enabled",
            ],
            &[],
            &["workflow execution", "trigger collection", "runtime recovery"],
            &[
                "prints recent runs, workflow logs, trigger events, and archive counts",
                "supports JSON output for automation and optional trigger/run filters",
            ],
            &[
                "resolve root config before reading runtime history",
                "respect the configured runtime storage backend without mutating active history",
            ],
            &[],
            &[
                "invalid `--limit` values are rejected with the exact argv position",
                "storage read failures surface as state errors without partial output",
            ],
            &[
                "chainbot observe",
                "chainbot observe --json --limit 5",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot observe --trigger-id tr-market",
            ],
            &["status", "list-runs", "serve", "catalog"],
        ),
        HelpTopic::Catalog => render_help_card(
            "catalog",
            "Discover builtin capabilities and installed plugin contracts",
            &[
                "chainbot catalog list [--json] [--kind <builtin_node|builtin_trigger|plugin>]",
                "chainbot catalog show <reference> [--json]",
            ],
            &[
                "you need builtin node or trigger inventory without opening source code",
                "you want installed plugin callable or event structure details",
                "you need to distinguish external trigger lifecycle: process_short_lived (poll-based) vs wasm_daemon_persistent_session (long-lived)",
                "an agent needs stable machine-readable capability discovery before generating config",
            ],
            &["builtin descriptors", "installed plugin manifests when a root is available"],
            &[],
            &[],
            &[
                "prints grouped human-readable capability lists by default",
                "prints stable JSON read models when `--json` is enabled",
                "external trigger plugins show lifecycle (process_short_lived or wasm_daemon_persistent_session) and runtime semantics",
                "wasm trigger plugins show host callback outcomes: retryable backpressure (queue_saturated/budget_exhausted) vs terminal lease_lost/shutting_down",
            ],
            &[
                "builtin descriptors are always available",
                "installed plugins are loaded from the resolved root when one exists",
            ],
            &[],
            &[
                "malformed references are rejected with the expected `<kind>:<value>` format",
                "unknown references direct you back to `chainbot catalog list`",
            ],
            &[
                "chainbot catalog list",
                "chainbot catalog list --json --kind plugin",
                "chainbot catalog show plugin:quote-node-plugin",
            ],
            &["help", "plugin", "status", "validate"],
        ),
        HelpTopic::Plugin => render_help_card(
            "plugin",
            "Discover and install remote plugins from github/git sources",
            &[
                "chainbot plugin source list github <owner>/<repo> [--ref <git-ref>] [--json]",
                "chainbot plugin source list git <remote> [--ref <git-ref>] [--json]",
                "chainbot plugin source show <github|git> <target> --plugin <plugin_id> [--ref <git-ref>] [--json]",
                "chainbot plugin install <github|git> <target> [--ref <git-ref>] [--plugin <plugin_id>] [--force]",
            ],
            &[
                "you want to inspect remote installable plugins before writing anything into the current root",
                "you want one operator-facing CLI surface for github and git plugin sources",
                "you need `--force` guarded replacement and rollback-aware plugin installation",
            ],
            &[
                "remote source repositories materialized into temporary workspaces",
                "current root config and plugin directory for install",
            ],
            &[
                "<root>/plugins/<plugin_id> when install succeeds",
                "temporary staging and backup directories during install",
            ],
            &[
                "remote plugin source inspection does not modify the current root",
                "catalog remains the installed-capability surface for the current root",
            ],
            &[
                "prints remote plugin summaries or details for source list/show",
                "prints installed target path, source, and replacement state for install",
            ],
            &[
                "source list/show do not resolve the current root",
                "install resolves the current root and re-validates it after swapping the plugin package",
            ],
            &[],
            &[
                "multi-plugin repositories require `--plugin <plugin_id>` for show/install",
                "existing installed plugins are preserved unless `--force` is supplied",
                "install failures report whether rollback completed successfully",
            ],
            &[
                "chainbot plugin source list github openai/example-plugins --json",
                "chainbot plugin source show git ../plugin-repo --plugin quote-node-plugin",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot plugin install git ../plugin-repo --plugin quote-node-plugin --force",
            ],
            &["catalog", "validate", "status"],
        ),
        HelpTopic::Stop => render_help_card(
            "stop",
            "Request graceful daemon shutdown",
            &["chainbot stop"],
            &[
                "you want the running daemon to stop without using kill manually",
                "you need an operator-safe lifecycle command for automation",
            ],
            &["persisted daemon session state"],
            &["persisted daemon stop request state"],
            &[],
            &[
                "prints whether a shutdown was requested or no daemon was running",
                "waits briefly for the daemon to release its lease",
            ],
            &[
                "resolve root config before inspecting or updating daemon state",
                "return success when the daemon is already inactive or stale",
            ],
            &[],
            &[
                "timeout paths surface a stable daemon stop error code",
                "invalid root config is reported before any stop request is written",
            ],
            &["chainbot stop", "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot stop"],
            &["serve", "status", "observe"],
        ),
        HelpTopic::Trigger => render_help_card(
            "trigger",
            "Inspect or persist trigger package state",
            &[
                "chainbot trigger list [--json]",
                "chainbot trigger enable <trigger-id>",
                "chainbot trigger disable <trigger-id>",
            ],
            &[
                "you need to inspect configured triggers without opening TOML manually",
                "you need to stop or re-enable a trigger without editing config by hand",
                "you want a machine-readable trigger inventory for automation",
            ],
            &["configured root config", "configured trigger packages"],
            &["target trigger package config.toml for enable or disable actions"],
            &["workflow runs", "trigger snapshots"],
            &[
                "prints a trigger table or JSON list for `list`",
                "prints the exact trigger config path touched by enable/disable",
            ],
            &[
                "resolve root config before loading trigger packages",
                "for `list`, only trigger package inputs are required beyond root config",
            ],
            &[("Trigger package example", "toml", TRIGGER_CONFIG_EXAMPLE)],
            &[
                "unsupported actions and extra arguments are reported with their exact argument position",
                "unknown trigger IDs direct you to `chainbot trigger list`",
            ],
            &[
                "chainbot trigger list",
                "chainbot trigger list --json",
                "chainbot trigger enable tr-market",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot trigger disable tr-market",
            ],
            &["init", "status", "validate", "serve", "catalog"],
        ),
        HelpTopic::Validate => render_help_card(
            "validate",
            "Validate config and package contracts",
            &["chainbot validate"],
            &[
                "you want to confirm a root is structurally valid",
                "you changed config and want a fast contract check before `run` or `serve`",
                "you want canonical examples for root, workflow, trigger, and plugin packages",
            ],
            &[
                "configured root config",
                "configured workflow packages",
                "configured trigger packages",
                "configured plugin packages",
            ],
            &[],
            &["workflow runs", "trigger snapshots"],
            &[
                "prints `validated root: <path>` on success",
                "prints file-aware validation diagnostics on failure",
            ],
            &[
                "load chainbot.toml first, then workflows, then triggers, then plugins",
                "reject absolute path overrides and any root-relative path that contains `..`",
            ],
            &[
                ("Root config example", "toml", ROOT_CONFIG_EXAMPLE),
                ("Workflow package example", "toml", WORKFLOW_CONFIG_EXAMPLE),
                ("Trigger package example", "toml", TRIGGER_CONFIG_EXAMPLE),
                ("Webhook trigger example", "toml", WEBHOOK_TRIGGER_CONFIG_EXAMPLE),
                ("WebSocket trigger example", "toml", WEBSOCKET_TRIGGER_CONFIG_EXAMPLE),
                ("Plugin package example", "toml", PLUGIN_CONFIG_EXAMPLE),
            ],
            &[
                "invalid TOML is reported with file path, line, column, and highlighted source",
                "semantic validation failures report the owning contract field or package identity",
            ],
            &[
                "chainbot validate",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot validate",
            ],
            &["status", "run", "serve", "catalog"],
        ),
        HelpTopic::ListRuns => render_help_card(
            "list-runs",
            "Print persisted run summaries as JSON",
            &["chainbot list-runs"],
            &[
                "you need machine-readable workflow run summaries",
                "you want raw persisted run status output without higher-level aggregation",
            ],
            &["configured state runs directory"],
            &[],
            &["workflow runs", "trigger snapshots"],
            &["prints a JSON array of persisted run summaries"],
            &[
                "resolve root config and runtime state before reading summaries",
                "runtime recovery is applied before listing persisted runs",
            ],
            &[],
            &[
                "unexpected flags are reported with their exact argument position",
                "serialization failures are surfaced as state errors",
            ],
            &[
                "chainbot list-runs",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot list-runs",
            ],
            &["status", "serve"],
        ),
        HelpTopic::Run => render_help_card(
            "run",
            "Execute one manual workflow run",
            &["chainbot run"],
            &[
                "you want a single manual execution without a serve lease",
                "your root contains exactly one workflow package or one inferable top-level workflow",
            ],
            &[
                "configured root config",
                "configured workflow packages",
                "configured plugin packages",
                "configured secrets directory",
            ],
            &[
                "configured state runs directory",
                "configured workflow log directory",
            ],
            &[],
            &[
                "prints run_id, workflow_id, and terminal status on success",
                "persists run summary and workflow log entries",
            ],
            &[
                "resolve root config before selecting a manual-run workflow",
                "manual run inference fails fast when multiple top-level workflows are present",
            ],
            &[("Workflow package example", "toml", WORKFLOW_CONFIG_EXAMPLE)],
            &[
                "top-level workflow inference failures are returned as usage errors",
                "execution failures surface the run_id so you can inspect state artifacts immediately",
            ],
            &[
                "chainbot run",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot run",
            ],
            &["status", "validate", "serve"],
        ),
        HelpTopic::Serve => render_help_card(
            "serve",
            "Start the background daemon control plane",
            &["chainbot serve"],
            &[
                "you want a long-running daemon that keeps trigger evaluation active",
                "you need a stable control-plane entrypoint for automation",
                "you want `status --json` and `observe` to read persisted daemon truth",
            ],
            &[
                "configured root config",
                "configured workflow packages",
                "configured trigger packages",
                "configured plugin packages",
                "configured secrets directory",
            ],
            &[
                "configured state runs directory",
                "configured workflow log directory",
                "configured trigger record directory",
                "configured coordination store",
            ],
            &[],
            &[
                "prints a start acknowledgement with daemon owner information",
                "returns conflict errors when another daemon already holds the lease",
            ],
            &[
                "acquire the daemon lease before spawning the background child",
                "the daemon reloads config and evaluates triggers on loop boundaries",
            ],
            &[
                ("Trigger package example", "toml", TRIGGER_CONFIG_EXAMPLE),
                ("Webhook trigger example", "toml", WEBHOOK_TRIGGER_CONFIG_EXAMPLE),
                ("WebSocket trigger example", "toml", WEBSOCKET_TRIGGER_CONFIG_EXAMPLE),
                ("Plugin package example", "toml", PLUGIN_CONFIG_EXAMPLE),
            ],
            &[
                "conflict paths expose a stable daemon-already-running error code",
                "daemon start failures release the preflight lease before returning",
            ],
            &[
                "chainbot serve",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot serve",
            ],
            &["status", "observe", "stop"],
        ),
    }
}

pub(super) fn render_version_output() -> String {
    format!("chainbot {CHAINBOT_VERSION}")
}
