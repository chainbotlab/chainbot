//! [INPUT]
//! Plugin source discoverability read models built from remote source repositories and install results.
//!
//! [OUTPUT]
//! Renders human-readable and machine-readable plugin source list/show and install outputs.
//!
//! [ROLE]
//! Owns CLI-facing formatting for remote plugin source discovery and install operations.

use crate::plugin::source::{
    InstallTransactionResult, PluginSourceDetail, PluginSourceListOutput, PluginSourceShowOutput,
};

pub(crate) fn render_plugin_source_list(output: &PluginSourceListOutput) -> String {
    let mut lines = vec![format!(
        "Plugin source list - {} {}",
        output.source.source_kind, output.source.target
    )];
    if let Some(git_ref) = output.source.git_ref.as_deref() {
        lines.push(format!("Requested ref: {git_ref}"));
    }
    if let Some(resolved_ref) = output.source.resolved_ref.as_deref() {
        lines.push(format!("Resolved ref: {resolved_ref}"));
    }
    lines.push(String::new());
    lines.push(String::from("Plugins:"));
    for plugin in &output.plugins {
        lines.push(format!(
            "  - plugin_id={} path={} summary={} kind={} runtime={} install_mode={} release_version={}",
            plugin.plugin_id,
            plugin.path,
            plugin.summary.as_deref().unwrap_or("—"),
            plugin.plugin_kind,
            plugin.runtime,
            plugin.install_mode,
            plugin.release_version.as_deref().unwrap_or("—")
        ));
        if !plugin.surfaces.is_empty() {
            lines.push(format!("    surfaces={}", plugin.surfaces.join(" | ")));
        }
    }
    lines.join("\n")
}

pub(crate) fn render_plugin_source_show(output: &PluginSourceShowOutput) -> String {
    let plugin = &output.plugin;
    let mut lines = vec![format!(
        "Plugin source show - {} {}",
        output.source.source_kind, output.source.target
    )];
    if let Some(git_ref) = output.source.git_ref.as_deref() {
        lines.push(format!("Requested ref: {git_ref}"));
    }
    if let Some(resolved_ref) = output.source.resolved_ref.as_deref() {
        lines.push(format!("Resolved ref: {resolved_ref}"));
    }
    lines.push(String::new());
    lines.push(render_plugin_detail(plugin));
    lines.join("\n")
}

fn render_plugin_detail(plugin: &PluginSourceDetail) -> String {
    let mut lines = vec![format!(
        "plugin_id={} path={} summary={} kind={} runtime={} install_mode={} release_version={}",
        plugin.plugin_id,
        plugin.path,
        plugin.summary.as_deref().unwrap_or("—"),
        plugin.plugin_kind,
        plugin.runtime,
        plugin.install_mode,
        plugin.release_version.as_deref().unwrap_or("—")
    )];
    lines.push(format!("entrypoint={}", plugin.entrypoint));
    lines.push(format!("entry_artifact={}", plugin.entry_artifact));
    lines.push(format!("capabilities={}", plugin.capabilities.join(", ")));
    if !plugin.surfaces.is_empty() {
        lines.push(format!("surfaces={}", plugin.surfaces.join(" | ")));
    }
    if let Some(build) = plugin.build.as_ref() {
        lines.push(format!(
            "build kind={} workdir={} command={}",
            build.kind,
            build.workdir,
            build.command.join(" ")
        ));
        for output in &build.outputs {
            lines.push(format!("  output from={} to={}", output.from, output.to));
        }
    }
    lines.join("\n")
}

pub(crate) fn render_plugin_install_success(
    locator_label: &str,
    result: &InstallTransactionResult,
    resolved_ref: Option<&str>,
) -> String {
    let mut lines = vec![format!(
        "plugin installed: plugin_id={} target={} source={} replaced_existing={}",
        result.plugin_id,
        result.target_dir.display(),
        locator_label,
        if result.replaced_existing {
            "yes"
        } else {
            "no"
        }
    )];
    if let Some(value) = resolved_ref {
        lines.push(format!("resolved_ref={value}"));
    }
    lines.join("\n")
}
