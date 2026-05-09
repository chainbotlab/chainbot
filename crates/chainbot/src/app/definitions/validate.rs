//! [INPUT]
//! Loaded workflow, trigger, and plugin definitions plus ingress-state validation helpers.
//!
//! [OUTPUT]
//! Returns bundle-level contract validation results covering identity uniqueness, package roots, and ingress compatibility.
//!
//! [ROLE]
//! Enforces cross-package application invariants before the runtime consumes a loaded root bundle.

use std::path::Path;

use crate::domain::trigger::{TriggerDefinition, TriggerKind};
use crate::domain::workflow::WorkflowDefinition;
use crate::errors::ContractError;
use crate::infrastructure::config::RootConfigDefinition;
use crate::ingress::build_desired_ingress_state;
use crate::plugin::{PluginActivationContract, PluginKind, PluginManifest};

const LEGACY_OFFICIAL_PLUGIN_IDS: &[&str] = &[
    "eth-node-official-plugin",
    "eth-trigger-official-plugin",
    "solana-node-official-plugin",
    "solana-trigger-official-plugin",
];

pub(crate) fn validate_bundle_contracts(
    root_config: &RootConfigDefinition,
    workflows: &[WorkflowDefinition],
    triggers: &[TriggerDefinition],
    plugins: &[PluginManifest],
) -> Result<(), ContractError> {
    let mut workflow_ids = std::collections::BTreeSet::new();
    for workflow in workflows {
        if !workflow_ids.insert(workflow.workflow_id.clone()) {
            return Err(ContractError::DuplicateWorkflowId {
                workflow_id: workflow.workflow_id.clone(),
            });
        }
        validate_package_identity("workflow", &workflow.package_root, &workflow.workflow_id)?;
    }

    let mut trigger_ids = std::collections::BTreeSet::new();
    for trigger in triggers {
        if !trigger_ids.insert(trigger.trigger_id.clone()) {
            return Err(ContractError::DuplicateTriggerId {
                trigger_id: trigger.trigger_id.clone(),
            });
        }
        validate_package_identity("trigger", &trigger.package_root, &trigger.trigger_id)?;
        if !workflow_ids.contains(&trigger.workflow_id) {
            return Err(ContractError::TriggerReferencesUnknownWorkflow {
                trigger_id: trigger.trigger_id.clone(),
                workflow_id: trigger.workflow_id.clone(),
            });
        }
    }

    let mut plugin_ids = std::collections::BTreeSet::new();
    let mut plugin_manifest_by_id = std::collections::BTreeMap::new();
    for plugin in plugins {
        if LEGACY_OFFICIAL_PLUGIN_IDS.contains(&plugin.plugin_id.as_str()) {
            return Err(ContractError::InvalidRootConfigField {
                field: "root_config.plugins",
                detail: format!(
                    "legacy official plugin_id {} is no longer supported; reinstall the package with its canonical short id",
                    plugin.plugin_id
                ),
            });
        }
        if !plugin_ids.insert(plugin.plugin_id.clone()) {
            return Err(ContractError::DuplicatePluginId {
                plugin_id: plugin.plugin_id.clone(),
            });
        }
        let plugin_package_root = plugin
            .manifest_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        validate_package_identity("plugin", &plugin_package_root, &plugin.plugin_id)?;
        plugin_manifest_by_id.insert(plugin.plugin_id.clone(), plugin);
    }

    for workflow in workflows {
        validate_legacy_builtin_http_usage(workflow)?;
    }

    for plugin_id in root_config.plugin_activation.keys() {
        if !plugin_ids.contains(plugin_id) {
            return Err(ContractError::InvalidRootConfigField {
                field: "root_config.plugin_activation",
                detail: format!(
                    "plugin_activation references unknown installed plugin_id {}",
                    plugin_id
                ),
            });
        }
    }

    for workflow in workflows {
        for node in &workflow.nodes {
            if node.kind != "plugin" {
                continue;
            }
            let Some(plugin) = plugin_manifest_by_id.get(&node.plugin_id) else {
                let detail = if node.plugin_id == "http-node" {
                    "workflow references plugin `http-node`, but it is not installed in this root; install the official package first, then re-run validation".to_owned()
                } else {
                    format!(
                        "workflow references plugin `{}`, but no installed plugin manifest was found for that plugin_id",
                        node.plugin_id
                    )
                };
                return Err(ContractError::InvalidWorkflowNodeField {
                    workflow_id: workflow.workflow_id.clone(),
                    node_id: node.node_id.clone(),
                    field: "node.plugin",
                    detail,
                });
            };
            if plugin.kind()? != PluginKind::ExternalNode {
                continue;
            }
            validate_plugin_activation_requirements(root_config, workflow, node, plugin)?;
        }
    }

    for trigger in triggers {
        if trigger.kind()? != TriggerKind::ExternalPlugin {
            continue;
        }
        let Some(plugin_id) = trigger.plugin.as_deref() else {
            continue;
        };
        let Some(plugin) = plugin_manifest_by_id.get(plugin_id) else {
            continue;
        };
        if plugin.kind()? != PluginKind::ExternalTrigger {
            continue;
        }
        validate_trigger_activation_requirements(root_config, trigger, plugin)?;
    }

    let _ = build_desired_ingress_state(triggers)?;

    Ok(())
}

fn validate_legacy_builtin_http_usage(workflow: &WorkflowDefinition) -> Result<(), ContractError> {
    for node in &workflow.nodes {
        if node.kind == "builtin.http" || node.plugin_id == "builtin.http" {
            return Err(ContractError::InvalidWorkflowNodeField {
                workflow_id: workflow.workflow_id.clone(),
                node_id: node.node_id.clone(),
                field: "node.plugin",
                detail: "builtin.http has been retired; install the official `http-node` plugin, switch to kind=`plugin`, use plugin=`http-node`, and set operation=`request`".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_plugin_activation_requirements(
    root_config: &RootConfigDefinition,
    workflow: &WorkflowDefinition,
    node: &crate::domain::runtime::NodeDefinition,
    plugin: &PluginManifest,
) -> Result<(), ContractError> {
    let Some(contract) = plugin.activation.as_ref() else {
        return Ok(());
    };
    let activation = root_config.plugin_activation.get(&plugin.plugin_id);

    if activation.is_none() {
        let requirement = describe_missing_activation_requirement(contract);
        if requirement.is_none() {
            return Ok(());
        }
        return Err(ContractError::InvalidWorkflowNodeField {
            workflow_id: workflow.workflow_id.clone(),
            node_id: node.node_id.clone(),
            field: "root_config.plugin_activation",
            detail: format!(
                "plugin `{}` requires plugin_activation because {requirement}",
                plugin.plugin_id,
                requirement = requirement.expect("checked above")
            ),
        });
    }

    let activation = activation.expect("checked above");
    validate_declared_activation_slots(workflow, node, plugin, contract, activation)?;
    if contract.requires_allowed_origins && activation.allowed_origins.is_empty() {
        return Err(ContractError::InvalidWorkflowNodeField {
            workflow_id: workflow.workflow_id.clone(),
            node_id: node.node_id.clone(),
            field: "root_config.plugin_activation.allowed_origins",
            detail: format!(
                "plugin `{}` requires allowed_origins",
                plugin.plugin_id
            ),
        });
    }
    Ok(())
}

fn validate_declared_activation_slots(
    workflow: &WorkflowDefinition,
    node: &crate::domain::runtime::NodeDefinition,
    plugin: &PluginManifest,
    contract: &PluginActivationContract,
    activation: &crate::infrastructure::config::PluginActivationDefinition,
) -> Result<(), ContractError> {
    for slot in activation.secret_bindings.keys() {
        let declared = contract
            .required_secret_slots
            .iter()
            .chain(contract.optional_secret_slots.iter())
            .any(|candidate| candidate == slot);
        if !declared {
            return Err(ContractError::InvalidWorkflowNodeField {
                workflow_id: workflow.workflow_id.clone(),
                node_id: node.node_id.clone(),
                field: "root_config.plugin_activation.secret_bindings",
                detail: format!(
                    "plugin `{}` does not declare activation slot `{}`",
                    plugin.plugin_id, slot
                ),
            });
        }
    }
    for slot in &contract.required_secret_slots {
        if !activation.secret_bindings.contains_key(slot) {
            return Err(ContractError::InvalidWorkflowNodeField {
                workflow_id: workflow.workflow_id.clone(),
                node_id: node.node_id.clone(),
                field: "root_config.plugin_activation.secret_bindings",
                detail: format!(
                    "plugin `{}` is missing required activation slot `{}`",
                    plugin.plugin_id, slot
                ),
            });
        }
    }
    Ok(())
}

fn validate_trigger_activation_requirements(
    root_config: &RootConfigDefinition,
    trigger: &TriggerDefinition,
    plugin: &PluginManifest,
) -> Result<(), ContractError> {
    let Some(contract) = plugin.activation.as_ref() else {
        return Ok(());
    };
    let activation = root_config.plugin_activation.get(&plugin.plugin_id);

    if activation.is_none() {
        let requirement = describe_missing_activation_requirement(contract);
        if requirement.is_none() {
            return Ok(());
        }
        return Err(ContractError::InvalidTriggerDefinitionField {
            trigger_id: trigger.trigger_id.clone(),
            field: "root_config.plugin_activation",
            detail: format!(
                "plugin `{}` requires plugin_activation because {requirement}",
                plugin.plugin_id,
                requirement = requirement.expect("checked above")
            ),
        });
    }

    let activation = activation.expect("checked above");
    for slot in activation.secret_bindings.keys() {
        let declared = contract
            .required_secret_slots
            .iter()
            .chain(contract.optional_secret_slots.iter())
            .any(|candidate| candidate == slot);
        if !declared {
            return Err(ContractError::InvalidTriggerDefinitionField {
                trigger_id: trigger.trigger_id.clone(),
                field: "root_config.plugin_activation.secret_bindings",
                detail: format!(
                    "plugin `{}` does not declare activation slot `{}`",
                    plugin.plugin_id, slot
                ),
            });
        }
    }
    for slot in &contract.required_secret_slots {
        if !activation.secret_bindings.contains_key(slot) {
            return Err(ContractError::InvalidTriggerDefinitionField {
                trigger_id: trigger.trigger_id.clone(),
                field: "root_config.plugin_activation.secret_bindings",
                detail: format!(
                    "plugin `{}` is missing required activation slot `{}`",
                    plugin.plugin_id, slot
                ),
            });
        }
    }
    if contract.requires_allowed_origins && activation.allowed_origins.is_empty() {
        return Err(ContractError::InvalidTriggerDefinitionField {
            trigger_id: trigger.trigger_id.clone(),
            field: "root_config.plugin_activation.allowed_origins",
            detail: format!(
                "plugin `{}` requires allowed_origins",
                plugin.plugin_id
            ),
        });
    }
    Ok(())
}

fn describe_missing_activation_requirement(contract: &PluginActivationContract) -> Option<String> {
    let mut parts = Vec::new();
    if !contract.required_secret_slots.is_empty() {
        parts.push(format!(
            "secret_bindings must provide required slots: {}",
            contract.required_secret_slots.join(", ")
        ));
    }
    if contract.requires_allowed_origins {
        parts.push("allowed_origins must be configured".to_owned());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

fn validate_package_identity(
    kind: &'static str,
    package_root: &Path,
    expected_id: &str,
) -> Result<(), ContractError> {
    let directory_name = package_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    if directory_name != expected_id {
        return Err(ContractError::PackageDirectoryIdentityMismatch {
            kind,
            path: package_root.to_path_buf(),
            expected_id: expected_id.to_owned(),
            directory_name,
        });
    }
    Ok(())
}
