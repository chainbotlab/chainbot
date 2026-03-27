//! [INPUT]
//! Loaded workflow, trigger, and plugin definitions plus ingress-state validation helpers.
//!
//! [OUTPUT]
//! Returns bundle-level contract validation results covering identity uniqueness, package roots, and ingress compatibility.
//!
//! [ROLE]
//! Enforces cross-package application invariants before the runtime consumes a loaded root bundle.

use std::path::Path;

use crate::domain::trigger::TriggerDefinition;
use crate::domain::workflow::WorkflowDefinition;
use crate::errors::ContractError;
use crate::ingress::build_desired_ingress_state;
use crate::plugin::PluginManifest;

pub(crate) fn validate_bundle_contracts(
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
    for plugin in plugins {
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
    }

    let _ = build_desired_ingress_state(triggers)?;

    Ok(())
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
