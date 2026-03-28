//! [INPUT]
//! Raw process arguments, CLI view filters, help text, and user-facing error contracts.
//!
//! [OUTPUT]
//! Parses argv into validated `CliRequest` values or usage-oriented `UserFacingError` results.
//!
//! [ROLE]
//! Converts the process command line into the application-layer request model.

use std::ffi::OsString;

use crate::app::cli::view::catalog::{CatalogFilterKind, CatalogReference};
use crate::errors::UserFacingError;
use crate::plugin::source::PluginSourceLocator;

use super::help::general_help_text;
use super::{
    CatalogRequest, CliCommand, CliRequest, HelpTopic, ObserveRequest, PluginCommand,
    PluginInstallRequest, PluginSourceRequest, TriggerOperation,
};

impl CliRequest {
    pub fn from_env() -> Result<Self, UserFacingError> {
        Self::parse_from_args(std::env::args_os().skip(1))
    }

    pub fn parse_from_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let Some(command_raw) = args.next() else {
            return Err(UserFacingError::usage(format!(
                "No command provided. Run `chainbot --help` to see available commands.\n\n{}",
                general_help_text()
            )));
        };

        let command_raw = command_raw.to_string_lossy().into_owned();
        match command_raw.as_str() {
            "-h" | "--help" => Ok(Self {
                command: CliCommand::Help(HelpTopic::General),
                json_output: false,
                trigger_operation: None,
                catalog_request: None,
                plugin_command: None,
                observe_request: None,
                daemon_owner_id: None,
            }),
            "-V" | "--version" => Ok(Self {
                command: CliCommand::Version,
                json_output: false,
                trigger_operation: None,
                catalog_request: None,
                plugin_command: None,
                observe_request: None,
                daemon_owner_id: None,
            }),
            "help" => Self::parse_help_args(args),
            "version" => Self::parse_command_args(CliCommand::Version, args),
            "init" => Self::parse_command_args(CliCommand::Init, args),
            "status" => Self::parse_command_args(CliCommand::Status, args),
            "observe" => Self::parse_observe_args(args),
            "catalog" => Self::parse_catalog_args(args),
            "plugin" => Self::parse_plugin_args(args),
            "stop" => Self::parse_command_args(CliCommand::Stop, args),
            "trigger" => Self::parse_trigger_args(args),
            "validate" => Self::parse_command_args(CliCommand::Validate, args),
            "run" => Self::parse_command_args(CliCommand::Run, args),
            "serve" => Self::parse_command_args(CliCommand::Serve, args),
            "__serve-daemon" => Self::parse_internal_daemon_args(args),
            "list-runs" => Self::parse_command_args(CliCommand::ListRuns, args),
            other => Err(UserFacingError::usage(format!(
                "{}",
                unsupported_command_message(other)
            ))),
        }
    }

    fn parse_help_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let topic = match args.next() {
            None => HelpTopic::General,
            Some(value) => parse_help_topic(&value.to_string_lossy())?,
        };

        if let Some(extra) = args.next() {
            return Err(UserFacingError::usage(format!(
                "Unexpected argument #2 for `chainbot help`: `{}`. Usage: `chainbot help [command]`.",
                extra.to_string_lossy()
            )));
        }

        Ok(Self {
            command: CliCommand::Help(topic),
            json_output: false,
            trigger_operation: None,
            catalog_request: None,
            plugin_command: None,
            observe_request: None,
            daemon_owner_id: None,
        })
    }

    fn parse_command_args<I>(command: CliCommand, mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let mut json_output = false;
        let mut position = 0usize;
        while let Some(arg) = args.next() {
            position += 1;
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "-h" | "--help" => {
                    return Ok(Self {
                        command: CliCommand::Help(help_topic_for(command)),
                        json_output: false,
                        trigger_operation: None,
                        catalog_request: None,
                        plugin_command: None,
                        observe_request: None,
                        daemon_owner_id: None,
                    });
                }
                "--json" if matches!(command, CliCommand::Status | CliCommand::Observe) => {
                    json_output = true;
                }
                _ => {
                    if let Some((flag, value)) = raw.split_once('=') {
                        if flag == "--json"
                            && matches!(command, CliCommand::Status | CliCommand::Observe)
                        {
                            json_output = parse_bool_flag_value(
                                "--json",
                                value,
                                position,
                                &format!("chainbot {}", command_name(command)),
                            )?;
                            continue;
                        }
                    }

                    return Err(UserFacingError::usage(format!(
                        "Unexpected argument #{position} after `chainbot {}`: `{raw}`. Run `chainbot help {}` for valid forms.",
                        command_name(command),
                        command_name(command)
                    )));
                }
            }
        }

        Ok(Self {
            command,
            json_output,
            trigger_operation: None,
            catalog_request: None,
            plugin_command: None,
            observe_request: None,
            daemon_owner_id: None,
        })
    }

    fn parse_internal_daemon_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let mut daemon_owner_id = None;
        let mut position = 0usize;

        while let Some(arg) = args.next() {
            position += 1;
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "--owner-id" => {
                    let Some(value) = args.next() else {
                        return Err(UserFacingError::usage_with_code(
                            "daemon_start_failed",
                            "`chainbot __serve-daemon --owner-id` requires a daemon owner identifier.",
                        ));
                    };
                    position += 1;
                    daemon_owner_id = Some(value.to_string_lossy().into_owned());
                }
                _ => {
                    if let Some((flag, value)) = raw.split_once('=') {
                        if flag == "--owner-id" {
                            daemon_owner_id = Some(value.to_owned());
                            continue;
                        }
                    }

                    return Err(UserFacingError::usage_with_code(
                        "daemon_start_failed",
                        format!(
                            "Unexpected argument #{position} after `chainbot __serve-daemon`: `{raw}`."
                        ),
                    ));
                }
            }
        }

        let daemon_owner_id = daemon_owner_id.ok_or_else(|| {
            UserFacingError::usage_with_code(
                "daemon_start_failed",
                "`chainbot __serve-daemon` requires `--owner-id <owner-id>`.",
            )
        })?;

        Ok(Self {
            command: CliCommand::InternalServeDaemon,
            json_output: false,
            trigger_operation: None,
            catalog_request: None,
            plugin_command: None,
            observe_request: None,
            daemon_owner_id: Some(daemon_owner_id),
        })
    }

    fn parse_catalog_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let Some(action) = args.next() else {
            return Err(UserFacingError::usage(
                "`chainbot catalog` requires `list` or `show`. Run `chainbot help catalog`.",
            ));
        };
        let action = action.to_string_lossy().into_owned();
        if matches!(action.as_str(), "-h" | "--help") {
            return Ok(Self {
                command: CliCommand::Help(HelpTopic::Catalog),
                json_output: false,
                trigger_operation: None,
                catalog_request: None,
                plugin_command: None,
                observe_request: None,
                daemon_owner_id: None,
            });
        }

        match action.as_str() {
            "list" => {
                let mut json_output = false;
                let mut filter = None;
                let mut position = 1usize;
                while let Some(arg) = args.next() {
                    position += 1;
                    let raw = arg.to_string_lossy().into_owned();
                    match raw.as_str() {
                        "--json" => json_output = true,
                        "--kind" => {
                            let Some(value) = args.next() else {
                                return Err(UserFacingError::usage(
                                    "`chainbot catalog list --kind` requires builtin_node, builtin_trigger, or plugin.",
                                ));
                            };
                            position += 1;
                            filter = Some(parse_catalog_filter_kind(
                                &value.to_string_lossy(),
                                position,
                            )?);
                        }
                        _ => {
                            if let Some((flag, value)) = raw.split_once('=') {
                                match flag {
                                    "--json" => {
                                        json_output = parse_bool_flag_value(
                                            "--json",
                                            value,
                                            position,
                                            "chainbot catalog list",
                                        )?;
                                        continue;
                                    }
                                    "--kind" => {
                                        filter = Some(parse_catalog_filter_kind(value, position)?);
                                        continue;
                                    }
                                    _ => {}
                                }
                            }
                            return Err(UserFacingError::usage(format!(
                                "Unexpected argument #{position} after `chainbot catalog list`: `{raw}`. Run `chainbot help catalog` for valid forms."
                            )));
                        }
                    }
                }

                Ok(Self {
                    command: CliCommand::Catalog,
                    json_output,
                    trigger_operation: None,
                    catalog_request: Some(CatalogRequest::List { filter }),
                    plugin_command: None,
                    observe_request: None,
                    daemon_owner_id: None,
                })
            }
            "show" => {
                let mut json_output = false;
                let mut reference = None;
                let mut position = 1usize;
                while let Some(arg) = args.next() {
                    position += 1;
                    let raw = arg.to_string_lossy().into_owned();
                    match raw.as_str() {
                        "--json" => json_output = true,
                        _ => {
                            if let Some((flag, value)) = raw.split_once('=') {
                                if flag == "--json" {
                                    json_output = parse_bool_flag_value(
                                        "--json",
                                        value,
                                        position,
                                        "chainbot catalog show",
                                    )?;
                                    continue;
                                }
                            }
                            if reference.is_none() {
                                reference = Some(CatalogReference::parse(&raw).map_err(|error| {
                                    UserFacingError::usage(format!(
                                        "Unsupported catalog reference at argument #{position} after `chainbot catalog show`: {error}. Run `chainbot catalog list` first."
                                    ))
                                })?);
                                continue;
                            }
                            return Err(UserFacingError::usage(format!(
                                "Unexpected argument #{position} after `chainbot catalog show`: `{raw}`. Run `chainbot help catalog` for valid forms."
                            )));
                        }
                    }
                }
                let reference = reference.ok_or_else(|| {
                    UserFacingError::usage(
                        "`chainbot catalog show` requires a <kind>:<value> reference. Run `chainbot catalog list`.",
                    )
                })?;

                Ok(Self {
                    command: CliCommand::Catalog,
                    json_output,
                    trigger_operation: None,
                    catalog_request: Some(CatalogRequest::Show { reference }),
                    plugin_command: None,
                    observe_request: None,
                    daemon_owner_id: None,
                })
            }
            other => Err(UserFacingError::usage(format!(
                "Unsupported catalog action at argument #1 after `chainbot catalog`: `{other}`. Use `list` or `show`."
            ))),
        }
    }

    fn parse_observe_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let mut json_output = false;
        let mut limit = 10usize;
        let mut trigger_id = None;
        let mut run_id = None;
        let mut position = 0usize;

        while let Some(arg) = args.next() {
            position += 1;
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "-h" | "--help" => {
                    return Ok(Self {
                        command: CliCommand::Help(HelpTopic::Observe),
                        json_output: false,
                        trigger_operation: None,
                        catalog_request: None,
                        plugin_command: None,
                        observe_request: None,
                        daemon_owner_id: None,
                    });
                }
                "--json" => {
                    json_output = true;
                }
                "--limit" => {
                    let Some(value) = args.next() else {
                        return Err(UserFacingError::usage(
                            "`chainbot observe --limit` requires a positive integer value.",
                        ));
                    };
                    position += 1;
                    limit = parse_observe_limit(&value.to_string_lossy(), position)?;
                }
                "--trigger-id" => {
                    let Some(value) = args.next() else {
                        return Err(UserFacingError::usage(
                            "`chainbot observe --trigger-id` requires a trigger identifier.",
                        ));
                    };
                    position += 1;
                    trigger_id = Some(value.to_string_lossy().into_owned());
                }
                "--run-id" => {
                    let Some(value) = args.next() else {
                        return Err(UserFacingError::usage(
                            "`chainbot observe --run-id` requires a run identifier.",
                        ));
                    };
                    position += 1;
                    run_id = Some(value.to_string_lossy().into_owned());
                }
                _ => {
                    if let Some((flag, value)) = raw.split_once('=') {
                        match flag {
                            "--json" => {
                                json_output = parse_bool_flag_value(
                                    "--json",
                                    value,
                                    position,
                                    "chainbot observe",
                                )?;
                                continue;
                            }
                            "--limit" => {
                                limit = parse_observe_limit(value, position)?;
                                continue;
                            }
                            "--trigger-id" => {
                                trigger_id = Some(value.to_owned());
                                continue;
                            }
                            "--run-id" => {
                                run_id = Some(value.to_owned());
                                continue;
                            }
                            _ => {}
                        }
                    }

                    return Err(UserFacingError::usage(format!(
                        "Unexpected argument #{position} after `chainbot observe`: `{raw}`. Run `chainbot help observe` for valid forms."
                    )));
                }
            }
        }

        Ok(Self {
            command: CliCommand::Observe,
            json_output,
            trigger_operation: None,
            catalog_request: None,
            plugin_command: None,
            observe_request: Some(ObserveRequest {
                limit,
                trigger_id,
                run_id,
            }),
            daemon_owner_id: None,
        })
    }

    fn parse_trigger_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let Some(action) = args.next() else {
            return Err(UserFacingError::usage(
                "`chainbot trigger` requires `list`, `enable`, or `disable`. Run `chainbot help trigger`.",
            ));
        };
        let action = action.to_string_lossy().into_owned();
        if matches!(action.as_str(), "-h" | "--help") {
            return Ok(Self {
                command: CliCommand::Help(HelpTopic::Trigger),
                json_output: false,
                trigger_operation: None,
                catalog_request: None,
                plugin_command: None,
                observe_request: None,
                daemon_owner_id: None,
            });
        }

        if action == "list" {
            let mut json_output = false;
            let mut position = 1usize;
            while let Some(arg) = args.next() {
                position += 1;
                let raw = arg.to_string_lossy().into_owned();
                match raw.as_str() {
                    "--json" => {
                        json_output = true;
                    }
                    _ => {
                        if let Some((flag, value)) = raw.split_once('=') {
                            if flag == "--json" {
                                json_output = parse_bool_flag_value(
                                    "--json",
                                    value,
                                    position,
                                    "chainbot trigger list",
                                )?;
                                continue;
                            }
                        }

                        return Err(UserFacingError::usage(format!(
                            "Unexpected argument #{position} after `chainbot trigger list`: `{raw}`. Run `chainbot help trigger` for valid forms."
                        )));
                    }
                }
            }

            return Ok(Self {
                command: CliCommand::Trigger,
                json_output,
                trigger_operation: Some(TriggerOperation::List),
                catalog_request: None,
                plugin_command: None,
                observe_request: None,
                daemon_owner_id: None,
            });
        }

        let enabled = match action.as_str() {
            "enable" => true,
            "disable" => false,
            other => {
                return Err(UserFacingError::usage(format!(
                    "Unsupported trigger action at argument #1 after `chainbot trigger`: `{other}`. Use `list`, `enable`, or `disable`."
                )));
            }
        };

        let mut trigger_id = None;
        let mut position = 1usize;
        while let Some(arg) = args.next() {
            position += 1;
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "-h" | "--help" => {
                    return Ok(Self {
                        command: CliCommand::Help(HelpTopic::Trigger),
                        json_output: false,
                        trigger_operation: None,
                        catalog_request: None,
                        plugin_command: None,
                        observe_request: None,
                        daemon_owner_id: None,
                    });
                }
                _ => {
                    if trigger_id.is_none() {
                        trigger_id = Some(raw);
                        continue;
                    }

                    return Err(UserFacingError::usage(format!(
                        "Unexpected argument #{position} after `chainbot trigger {}`: `{raw}`. Run `chainbot help trigger` for valid forms.",
                        if enabled { "enable" } else { "disable" }
                    )));
                }
            }
        }

        let trigger_id = trigger_id.ok_or_else(|| {
            UserFacingError::usage(
                "`chainbot trigger enable|disable` requires a <trigger-id>. Run `chainbot help trigger`.",
            )
        })?;

        Ok(Self {
            command: CliCommand::Trigger,
            json_output: false,
            trigger_operation: Some(if enabled {
                TriggerOperation::Enable { trigger_id }
            } else {
                TriggerOperation::Disable { trigger_id }
            }),
            catalog_request: None,
            plugin_command: None,
            observe_request: None,
            daemon_owner_id: None,
        })
    }

    fn parse_plugin_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let Some(action) = args.next() else {
            return Err(UserFacingError::usage(
                "`chainbot plugin` requires `source` or `install`. Run `chainbot help plugin`.",
            ));
        };
        let action = action.to_string_lossy().into_owned();
        if matches!(action.as_str(), "-h" | "--help") {
            return Ok(Self {
                command: CliCommand::Help(HelpTopic::Plugin),
                json_output: false,
                trigger_operation: None,
                catalog_request: None,
                plugin_command: None,
                observe_request: None,
                daemon_owner_id: None,
            });
        }

        match action.as_str() {
            "source" => Self::parse_plugin_source_args(args),
            "install" => Self::parse_plugin_install_args(args),
            other => Err(UserFacingError::usage(format!(
                "Unsupported action after `chainbot plugin`: `{other}`. Run `chainbot help plugin`."
            ))),
        }
    }

    fn parse_plugin_source_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let Some(action) = args.next() else {
            return Err(UserFacingError::usage(
                "`chainbot plugin source` requires `list` or `show`. Run `chainbot help plugin`.",
            ));
        };
        match action.to_string_lossy().as_ref() {
            "list" => {
                let (locator, json_output, plugin_id, _force) =
                    parse_plugin_locator_flags_with_force(args)?;
                if plugin_id.is_some() {
                    return Err(UserFacingError::usage(
                        "`chainbot plugin source list` does not accept `--plugin`. Run `chainbot help plugin`.",
                    ));
                }
                Ok(Self {
                    command: CliCommand::Plugin,
                    json_output,
                    trigger_operation: None,
                    catalog_request: None,
                    plugin_command: Some(PluginCommand::Source(PluginSourceRequest::List {
                        locator,
                    })),
                    observe_request: None,
                    daemon_owner_id: None,
                })
            }
            "show" => {
                let (locator, json_output, plugin_id, _force) =
                    parse_plugin_locator_flags_with_force(args)?;
                Ok(Self {
                    command: CliCommand::Plugin,
                    json_output,
                    trigger_operation: None,
                    catalog_request: None,
                    plugin_command: Some(PluginCommand::Source(PluginSourceRequest::Show {
                        locator,
                        plugin_id,
                    })),
                    observe_request: None,
                    daemon_owner_id: None,
                })
            }
            other => Err(UserFacingError::usage(format!(
                "Unsupported action after `chainbot plugin source`: `{other}`. Use `list` or `show`."
            ))),
        }
    }

    fn parse_plugin_install_args<I>(args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let (locator, _json_output, plugin_id, force) = parse_plugin_locator_flags_with_force(args)?;
        Ok(Self {
            command: CliCommand::Plugin,
            json_output: false,
            trigger_operation: None,
            catalog_request: None,
            plugin_command: Some(PluginCommand::Install(PluginInstallRequest {
                locator,
                plugin_id,
                force,
            })),
            observe_request: None,
            daemon_owner_id: None,
        })
    }
}

fn parse_plugin_locator_flags_with_force<I>(
    mut args: I,
) -> Result<(PluginSourceLocator, bool, Option<String>, bool), UserFacingError>
where
    I: Iterator<Item = OsString>,
{
    let Some(source_kind) = args.next() else {
        return Err(UserFacingError::usage(
            "plugin commands require a source kind: `github` or `git`.",
        ));
    };
    let source_kind = source_kind.to_string_lossy().into_owned();
    let Some(target) = args.next() else {
        return Err(UserFacingError::usage(format!(
            "`chainbot plugin ... {source_kind}` requires a source target."
        )));
    };
    let target = target.to_string_lossy().into_owned();
    let mut git_ref = None;
    let mut plugin_id = None;
    let mut json_output = false;
    let mut force = false;
    while let Some(arg) = args.next() {
        let raw = arg.to_string_lossy().into_owned();
        match raw.as_str() {
            "--json" => json_output = true,
            "--force" => force = true,
            "--ref" => {
                let Some(value) = args.next() else {
                    return Err(UserFacingError::usage(
                        "`--ref` requires `<git-ref>`. Run `chainbot help plugin`.",
                    ));
                };
                git_ref = Some(value.to_string_lossy().into_owned());
            }
            "--plugin" => {
                let Some(value) = args.next() else {
                    return Err(UserFacingError::usage(
                        "`--plugin` requires `<plugin_id>`. Run `chainbot help plugin`.",
                    ));
                };
                plugin_id = Some(value.to_string_lossy().into_owned());
            }
            _ => {
                if let Some((flag, value)) = raw.split_once('=') {
                    match flag {
                        "--ref" => git_ref = Some(value.to_owned()),
                        "--plugin" => plugin_id = Some(value.to_owned()),
                        "--json" => {
                            json_output = parse_bool_flag_value(
                                "--json",
                                value,
                                0,
                                "chainbot plugin",
                            )?
                        }
                        "--force" => {
                            force = parse_bool_flag_value(
                                "--force",
                                value,
                                0,
                                "chainbot plugin",
                            )?
                        }
                        _ => {
                            return Err(UserFacingError::usage(format!(
                                "Unexpected plugin argument: `{raw}`. Run `chainbot help plugin`."
                            )))
                        }
                    }
                    continue;
                }
                return Err(UserFacingError::usage(format!(
                    "Unexpected plugin argument: `{raw}`. Run `chainbot help plugin`."
                )));
            }
        }
    }

    let locator = match source_kind.as_str() {
        "github" => {
            let (owner, repo) = target.split_once('/').ok_or_else(|| {
                UserFacingError::usage(
                    "`chainbot plugin ... github` requires `<owner>/<repo>` as the target.",
                )
            })?;
            PluginSourceLocator::GitHub {
                owner: owner.to_owned(),
                repo: repo.to_owned(),
                git_ref,
            }
        }
        "git" => PluginSourceLocator::Git { remote: target, git_ref },
        other => {
            return Err(UserFacingError::usage(format!(
                "Unsupported plugin source kind `{other}`. Use `github` or `git`."
            )))
        }
    };

    Ok((locator, json_output, plugin_id, force))
}

fn parse_help_topic(value: &str) -> Result<HelpTopic, UserFacingError> {
    match value {
        "version" => Ok(HelpTopic::Version),
        "init" => Ok(HelpTopic::Init),
        "status" => Ok(HelpTopic::Status),
        "observe" => Ok(HelpTopic::Observe),
        "catalog" => Ok(HelpTopic::Catalog),
        "plugin" => Ok(HelpTopic::Plugin),
        "stop" => Ok(HelpTopic::Stop),
        "trigger" => Ok(HelpTopic::Trigger),
        "validate" => Ok(HelpTopic::Validate),
        "run" => Ok(HelpTopic::Run),
        "serve" => Ok(HelpTopic::Serve),
        "list-runs" => Ok(HelpTopic::ListRuns),
        other => Err(UserFacingError::usage(format!(
            "Unsupported help topic at argument #1 after `chainbot help`: `{other}`. Run `chainbot help` to see available command skills."
        ))),
    }
}

fn help_topic_for(command: CliCommand) -> HelpTopic {
    match command {
        CliCommand::Help(topic) => topic,
        CliCommand::Version => HelpTopic::Version,
        CliCommand::Init => HelpTopic::Init,
        CliCommand::Status => HelpTopic::Status,
        CliCommand::Observe => HelpTopic::Observe,
        CliCommand::Catalog => HelpTopic::Catalog,
        CliCommand::Plugin => HelpTopic::Plugin,
        CliCommand::Stop => HelpTopic::Stop,
        CliCommand::Trigger => HelpTopic::Trigger,
        CliCommand::Validate => HelpTopic::Validate,
        CliCommand::Run => HelpTopic::Run,
        CliCommand::Serve => HelpTopic::Serve,
        CliCommand::InternalServeDaemon => HelpTopic::Serve,
        CliCommand::ListRuns => HelpTopic::ListRuns,
    }
}

fn command_name(command: CliCommand) -> &'static str {
    match command {
        CliCommand::Help(_) => "help",
        CliCommand::Version => "version",
        CliCommand::Init => "init",
        CliCommand::Status => "status",
        CliCommand::Observe => "observe",
        CliCommand::Catalog => "catalog",
        CliCommand::Plugin => "plugin",
        CliCommand::Stop => "stop",
        CliCommand::Trigger => "trigger",
        CliCommand::Validate => "validate",
        CliCommand::Run => "run",
        CliCommand::Serve => "serve",
        CliCommand::InternalServeDaemon => "__serve-daemon",
        CliCommand::ListRuns => "list-runs",
    }
}

fn parse_catalog_filter_kind(
    value: &str,
    position: usize,
) -> Result<CatalogFilterKind, UserFacingError> {
    CatalogFilterKind::parse(value).ok_or_else(|| {
        UserFacingError::usage(format!(
            "Unsupported --kind value at argument #{position} after `chainbot catalog list`: `{value}`. Use builtin_node, builtin_trigger, or plugin."
        ))
    })
}

fn parse_bool_flag_value(
    flag_name: &str,
    value: &str,
    position: usize,
    command_path: &str,
) -> Result<bool, UserFacingError> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(UserFacingError::usage(format!(
            "Unsupported {flag_name} value at argument #{position} after `{command_path}`: `{other}`. Use true, false, 1, or 0."
        ))),
    }
}

fn parse_observe_limit(value: &str, position: usize) -> Result<usize, UserFacingError> {
    let parsed = value.parse::<usize>().map_err(|_| {
        UserFacingError::usage(format!(
            "Unsupported --limit value at argument #{position} after `chainbot observe`: `{value}`. Use a positive integer."
        ))
    })?;
    if parsed == 0 {
        return Err(UserFacingError::usage(format!(
            "Unsupported --limit value at argument #{position} after `chainbot observe`: `{value}`. Use a positive integer."
        )));
    }
    Ok(parsed)
}

fn unsupported_command_message(value: &str) -> String {
    match suggest_command(value) {
        Some(suggestion) => format!(
            "Unsupported command at argv[1]: `{value}`. Did you mean `{suggestion}`? Run `chainbot help` to see available command skills."
        ),
        None => format!(
            "Unsupported command at argv[1]: `{value}`. Run `chainbot help` to see available command skills."
        ),
    }
}

fn suggest_command(value: &str) -> Option<&'static str> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return None;
    }

    let mut best_match = None;
    let mut best_distance = usize::MAX;

    for candidate in [
        "help",
        "version",
        "status",
        "observe",
        "plugin",
        "init",
        "trigger",
        "validate",
        "list-runs",
        "run",
        "serve",
    ] {
        let distance = levenshtein_distance(normalized, candidate);
        if distance < best_distance {
            best_distance = distance;
            best_match = Some(candidate);
        }
    }

    match (best_match, best_distance) {
        (Some(candidate), distance) if distance <= 3 => Some(candidate),
        _ => None,
    }
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }

    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut costs = (0..=right_chars.len()).collect::<Vec<_>>();

    for (left_index, left_char) in left_chars.iter().enumerate() {
        let mut previous_diagonal = costs[0];
        costs[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let insertion = costs[right_index + 1] + 1;
            let deletion = costs[right_index] + 1;
            let substitution = previous_diagonal + usize::from(left_char != right_char);
            previous_diagonal = costs[right_index + 1];
            costs[right_index + 1] = insertion.min(deletion).min(substitution);
        }
    }

    costs[right_chars.len()]
}
