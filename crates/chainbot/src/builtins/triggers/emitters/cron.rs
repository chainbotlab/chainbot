use serde::Deserialize;
use time::OffsetDateTime;

use crate::builtins::triggers::context::BuiltinTriggerContext;
use crate::builtins::triggers::contract::{decode_builtin_trigger_params, BuiltinTriggerHandler};
use crate::errors::ContractError;
use crate::trigger::{TriggerDefinition, TriggerEmission, TRIGGER_KIND_CRON_ALIAS};

const MILLIS_PER_MINUTE: i64 = 60_000;
pub(crate) const BUILTIN_TRIGGER_CRON_KIND: &str = TRIGGER_KIND_CRON_ALIAS;

#[derive(Debug, Clone, Copy)]
pub struct CronTriggerHandler;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CronTriggerParams {
    schedule: String,
    #[serde(default)]
    timezone: Option<String>,
}

#[derive(Debug, Clone)]
struct CronSchedule {
    minute: CronField,
    hour: CronField,
    day: CronField,
    month: CronField,
    weekday: CronField,
}

#[derive(Debug, Clone)]
struct CronField {
    range_start: u8,
    range_end: u8,
    patterns: Vec<CronPattern>,
}

#[derive(Debug, Clone, Copy)]
struct CronPattern {
    start: u8,
    end: u8,
    step: u8,
}

#[derive(Debug, Clone, Copy)]
struct UtcCronSlot {
    minute: u8,
    hour: u8,
    day: u8,
    month: u8,
    weekday: u8,
}

impl BuiltinTriggerHandler for CronTriggerHandler {
    fn kind(&self) -> &str {
        BUILTIN_TRIGGER_CRON_KIND
    }

    fn validate(&self, definition: &TriggerDefinition) -> Result<(), ContractError> {
        let _ = parse_cron_params(definition)?;
        Ok(())
    }

    fn emit(
        &self,
        context: &BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        let params = parse_cron_params(definition)?;
        let slot_start_ms = context.now_ms.div_euclid(MILLIS_PER_MINUTE) * MILLIS_PER_MINUTE;
        let slot = slot_from_epoch_ms(definition, slot_start_ms)?;
        if !params.schedule.matches(slot) {
            return Ok(Vec::new());
        }

        let event_id = format!("cron:{}:{}", definition.trigger_id, slot_start_ms);
        Ok(vec![TriggerEmission {
            event_id: event_id.clone(),
            occurred_at_ms: slot_start_ms,
            source: Some(definition.source.clone()),
            payload: serde_json::json!({
                "kind": BUILTIN_TRIGGER_CRON_KIND,
                "source": definition.source,
                "schedule": params.raw_schedule,
                "slot_start_ms": slot_start_ms,
                "timezone": "UTC",
            }),
            dedup_key: Some(event_id),
            dedup_window_ms: Some(MILLIS_PER_MINUTE),
            cooldown_key: None,
            cooldown_ms: None,
        }])
    }
}

struct ParsedCronTriggerParams {
    raw_schedule: String,
    schedule: CronSchedule,
}

fn parse_cron_params(
    definition: &TriggerDefinition,
) -> Result<ParsedCronTriggerParams, ContractError> {
    let params: CronTriggerParams = decode_builtin_trigger_params(definition)?;
    match params.timezone.as_deref() {
        None | Some("UTC") => {}
        Some(timezone) => {
            return Err(ContractError::InvalidTriggerDefinitionField {
                trigger_id: definition.trigger_id.clone(),
                field: "trigger.params.timezone",
                detail: format!(
                    "unsupported timezone `{timezone}`; builtin cron currently supports UTC only"
                ),
            });
        }
    }

    let schedule = CronSchedule::parse(definition, &params.schedule)?;
    Ok(ParsedCronTriggerParams {
        raw_schedule: params.schedule,
        schedule,
    })
}

impl CronSchedule {
    fn parse(definition: &TriggerDefinition, expression: &str) -> Result<Self, ContractError> {
        let segments = expression.split_whitespace().collect::<Vec<_>>();
        if segments.len() != 5 {
            return Err(invalid_schedule(
                definition,
                format!("expected 5 cron fields but found {}", segments.len()),
            ));
        }

        Ok(Self {
            minute: CronField::parse(definition, segments[0], "minute", 0, 59, false)?,
            hour: CronField::parse(definition, segments[1], "hour", 0, 23, false)?,
            day: CronField::parse(definition, segments[2], "day", 1, 31, false)?,
            month: CronField::parse(definition, segments[3], "month", 1, 12, false)?,
            weekday: CronField::parse(definition, segments[4], "weekday", 0, 6, true)?,
        })
    }

    fn matches(&self, slot: UtcCronSlot) -> bool {
        let minute_matches = self.minute.matches(slot.minute);
        let hour_matches = self.hour.matches(slot.hour);
        let month_matches = self.month.matches(slot.month);
        let day_matches = self.day.matches(slot.day);
        let weekday_matches = self.weekday.matches(slot.weekday);
        let day_or_weekday_matches = match (self.day.is_wildcard(), self.weekday.is_wildcard()) {
            (true, true) => true,
            (true, false) => weekday_matches,
            (false, true) => day_matches,
            (false, false) => day_matches || weekday_matches,
        };

        minute_matches && hour_matches && month_matches && day_or_weekday_matches
    }
}

impl CronField {
    fn parse(
        definition: &TriggerDefinition,
        segment: &str,
        label: &str,
        range_start: u8,
        range_end: u8,
        allow_weekday_seven: bool,
    ) -> Result<Self, ContractError> {
        let patterns = segment
            .split(',')
            .map(|token| {
                CronPattern::parse(
                    definition,
                    token,
                    label,
                    range_start,
                    range_end,
                    allow_weekday_seven,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            range_start,
            range_end,
            patterns,
        })
    }

    fn matches(&self, value: u8) -> bool {
        self.patterns
            .iter()
            .any(|pattern| pattern.matches(value, self.range_start, self.range_end))
    }

    fn is_wildcard(&self) -> bool {
        self.patterns.len() == 1
            && self.patterns[0].start == self.range_start
            && self.patterns[0].end == self.range_end
            && self.patterns[0].step == 1
    }
}

impl CronPattern {
    fn parse(
        definition: &TriggerDefinition,
        token: &str,
        label: &str,
        range_start: u8,
        range_end: u8,
        allow_weekday_seven: bool,
    ) -> Result<Self, ContractError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(invalid_schedule(
                definition,
                format!("empty token in {label} field"),
            ));
        }

        let (base, step) = match token.split_once('/') {
            Some((base, step)) => (base, parse_step(definition, step, label)?),
            None => (token, 1),
        };

        if base == "*" {
            return Ok(Self {
                start: range_start,
                end: range_end,
                step,
            });
        }

        if let Some((start, end)) = base.split_once('-') {
            let start = parse_value(
                definition,
                start,
                label,
                range_start,
                range_end,
                allow_weekday_seven,
            )?;
            let end = parse_value(
                definition,
                end,
                label,
                range_start,
                range_end,
                allow_weekday_seven,
            )?;
            if start > end {
                return Err(invalid_schedule(
                    definition,
                    format!("invalid {label} range `{base}`; start must be <= end"),
                ));
            }
            return Ok(Self { start, end, step });
        }

        if step != 1 {
            return Err(invalid_schedule(
                definition,
                format!("invalid {label} token `{token}`; step syntax requires `*` or a range"),
            ));
        }

        let exact = parse_value(
            definition,
            base,
            label,
            range_start,
            range_end,
            allow_weekday_seven,
        )?;
        Ok(Self {
            start: exact,
            end: exact,
            step: 1,
        })
    }

    fn matches(&self, value: u8, range_start: u8, range_end: u8) -> bool {
        if value < range_start || value > range_end || value < self.start || value > self.end {
            return false;
        }

        (value - self.start) % self.step == 0
    }
}

fn slot_from_epoch_ms(
    definition: &TriggerDefinition,
    epoch_ms: i64,
) -> Result<UtcCronSlot, ContractError> {
    let timestamp = OffsetDateTime::from_unix_timestamp_nanos(i128::from(epoch_ms) * 1_000_000)
        .map_err(|source| ContractError::InvalidTriggerDefinitionField {
            trigger_id: definition.trigger_id.clone(),
            field: "trigger.params.schedule",
            detail: format!("invalid UTC slot timestamp: {source}"),
        })?;

    Ok(UtcCronSlot {
        minute: timestamp.minute(),
        hour: timestamp.hour(),
        day: timestamp.day(),
        month: u8::from(timestamp.month()),
        weekday: timestamp.weekday().number_days_from_sunday(),
    })
}

fn parse_step(
    definition: &TriggerDefinition,
    raw_step: &str,
    label: &str,
) -> Result<u8, ContractError> {
    let step = raw_step
        .parse::<u8>()
        .map_err(|_| invalid_schedule(definition, format!("invalid {label} step `{raw_step}`")))?;
    if step == 0 {
        return Err(invalid_schedule(
            definition,
            format!("invalid {label} step `{raw_step}`; step must be greater than zero"),
        ));
    }
    Ok(step)
}

fn parse_value(
    definition: &TriggerDefinition,
    raw_value: &str,
    label: &str,
    range_start: u8,
    range_end: u8,
    allow_weekday_seven: bool,
) -> Result<u8, ContractError> {
    let mut value = raw_value.parse::<u8>().map_err(|_| {
        invalid_schedule(definition, format!("invalid {label} value `{raw_value}`"))
    })?;
    if allow_weekday_seven && value == 7 {
        value = 0;
    }
    if value < range_start || value > range_end {
        return Err(invalid_schedule(
            definition,
            format!("invalid {label} value `{raw_value}`; expected {range_start}-{range_end}"),
        ));
    }
    Ok(value)
}

fn invalid_schedule(definition: &TriggerDefinition, detail: String) -> ContractError {
    ContractError::InvalidTriggerDefinitionField {
        trigger_id: definition.trigger_id.clone(),
        field: "trigger.params.schedule",
        detail,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;

    fn cron_definition(schedule: &str) -> TriggerDefinition {
        TriggerDefinition {
            api_version: String::from("2.0.0"),
            trigger_id: String::from("cron-trigger"),
            kind: String::from("builtin"),
            source: String::from(BUILTIN_TRIGGER_CRON_KIND),
            plugin: None,
            workflow_id: String::from("wf-cron"),
            enabled: true,
            params: BTreeMap::from([(String::from("schedule"), serde_json::json!(schedule))]),
            input_mapping: BTreeMap::new(),
            package_root: PathBuf::new(),
        }
    }

    #[test]
    fn cron_handler_rejects_invalid_schedule() {
        let error = CronTriggerHandler
            .validate(&cron_definition("* * *"))
            .expect_err("invalid cron schedule should fail validation");

        assert!(error.to_string().contains("expected 5 cron fields"));
    }

    #[test]
    fn cron_handler_emits_stable_slot_event_when_due() {
        let definition = cron_definition("*/15 * * * *");
        let emissions = CronTriggerHandler
            .emit(
                &BuiltinTriggerContext {
                    now_ms: 1_736_172_900_123,
                },
                &definition,
            )
            .expect("cron emission should succeed when schedule is due");

        assert_eq!(emissions.len(), 1);
        assert_eq!(emissions[0].occurred_at_ms, 1_736_172_900_000);
        assert_eq!(emissions[0].event_id, "cron:cron-trigger:1736172900000");
    }

    #[test]
    fn cron_handler_skips_non_matching_slot() {
        let definition = cron_definition("0 * * * *");
        let emissions = CronTriggerHandler
            .emit(
                &BuiltinTriggerContext {
                    now_ms: 1_736_172_900_123,
                },
                &definition,
            )
            .expect("cron emission should succeed");

        assert!(emissions.is_empty());
    }
}
