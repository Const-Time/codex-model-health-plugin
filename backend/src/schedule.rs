//! Durable, serial, opt-in account diagnostics with bounded retries and persisted progress.
use crate::{Service, host, invalid, now_ms};
use gateway_plugin_sdk::{PluginFault, call::host::*, client::HostClient};
use serde::{Deserialize, Serialize};
use serde_json::json;
mod detection;
mod timing;
pub(crate) use detection::Detection;
use timing::Timing;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Schedule {
    pub enabled: bool,
    pub model: String,
    pub interval_minutes: u64,
    #[serde(default)]
    pub timing: Timing,
    pub accounts: Vec<Entry>,
    #[serde(default)]
    pub detection: Detection,
    #[serde(default)]
    pub active_account: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub account_id: String,
    pub name: String,
    pub next_at: u64,
    pub status: String,
    pub checked_at: Option<u64>,
    pub message: String,
    pub pending: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub failures: u8,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Configure {
    enabled: bool,
    model: String,
    interval_minutes: u64,
    #[serde(default)]
    timing: Timing,
    #[serde(default)]
    detection: Detection,
    account_ids: Vec<String>,
    #[serde(default)]
    resume_accounts: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preview {
    interval_minutes: u64,
    timing: Timing,
}
pub(crate) fn preview(input: Preview) -> Result<serde_json::Value, PluginFault> {
    let prepared = input.timing.prepare(input.interval_minutes)?;
    let mut at = now_ms();
    let mut next = Vec::new();
    for _ in 0..3 {
        at = prepared.next_after(at)?;
        next.push(at);
    }
    Ok(json!({"next_at":next,"timezone":input.timing.timezone}))
}

pub(crate) fn migrate(input: StateMigrationRequest) -> Result<StateMigrationResult, PluginFault> {
    if input.namespace == "runs" && input.from_schema_version == 1 && input.to_schema_version == 2 {
        return Ok(StateMigrationResult {
            changes: input
                .records
                .into_iter()
                .map(|record| StateMigrationChange::Keep { key: record.key })
                .collect(),
        });
    }
    if input.namespace != "schedule"
        || ![1, 2].contains(&input.from_schema_version)
        || input.to_schema_version != 3
    {
        return Err(invalid("不支持此计划状态迁移"));
    }
    let mut changes = Vec::new();
    for record in input.records {
        if record.key != "settings" {
            return Err(invalid("计划状态键无效"));
        }
        let mut value = record.value;
        if !value.is_object() {
            return Err(invalid("旧计划状态无效"));
        }
        // Preserve every account's pending marker, status, due time and diagnostic history.
        if input.from_schema_version == 1 {
            value["timing"] =
                serde_json::to_value(Timing::default()).map_err(|_| invalid("计划编码失败"))?;
        }
        value["detection"] =
            serde_json::to_value(Detection::default()).map_err(|_| invalid("计划编码失败"))?;
        value["active_account"] = serde_json::Value::Null;
        let _: Schedule =
            serde_json::from_value(value.clone()).map_err(|_| invalid("旧计划状态无效"))?;
        changes.push(StateMigrationChange::Replace {
            key: record.key,
            value,
        });
    }
    Ok(StateMigrationResult { changes })
}
pub(crate) async fn read(host: &HostClient) -> Result<(Schedule, Option<u64>), PluginFault> {
    let result: StateGetResult = host::metadata(
        host,
        "host.state.get",
        StateGetRequest {
            namespace: "schedule".into(),
            key: "settings".into(),
        },
    )
    .await?;
    match result.record {
        Some(record) => Ok((
            serde_json::from_value(record.value).map_err(|_| invalid("定时计划状态损坏"))?,
            Some(record.version),
        )),
        None => Ok((
            Schedule {
                interval_minutes: 360,
                detection: Detection::new_detection(),
                ..Schedule::default()
            },
            None,
        )),
    }
}
async fn write(
    host: &HostClient,
    plan: &Schedule,
    version: Option<u64>,
) -> Result<(), PluginFault> {
    let _: StatePutResult = host::metadata(
        host,
        "host.state.put",
        StatePutRequest {
            namespace: "schedule".into(),
            key: "settings".into(),
            value: serde_json::to_value(plan).map_err(|_| invalid("计划编码失败"))?,
            expected_version: version,
        },
    )
    .await?;
    Ok(())
}
pub(crate) async fn save(
    service: &Service,
    host: &HostClient,
    input: Configure,
) -> Result<Schedule, PluginFault> {
    if !(60..=10080).contains(&input.interval_minutes)
        || input.account_ids.len() > 50
        || (input.enabled && (input.account_ids.is_empty() || input.model.trim().is_empty()))
        || input.model.len() > 128
        || input.model.chars().any(char::is_control)
    {
        return Err(invalid(
            "间隔需为 60–10080 分钟，最多选择 50 个账号，启用时必须填写模型",
        ));
    }
    let next_at = input
        .timing
        .prepare(input.interval_minutes)?
        .next_after(now_ms())?;
    input
        .detection
        .validate(service, host, &input.model, input.enabled)
        .await?;
    let available = if input.enabled {
        host::accounts(host).await?
    } else {
        vec![]
    };
    let _lock = service.state.lock().await;
    let (old, version) = read(host).await?;
    if old.active_account.is_some()
        && input.enabled
        && (old.detection != input.detection || old.model != input.model.trim())
    {
        return Err(invalid(
            "已有定时检测正在进行，请先关闭计划再修改模型或检测方法",
        ));
    }
    if old
        .active_account
        .as_ref()
        .is_some_and(|id| !input.account_ids.contains(id))
    {
        return Err(invalid("请先关闭计划并保存，再移除正在检测的账号"));
    }
    if !input.enabled {
        detection::pause_active(service, host, &old).await?;
    }
    if old
        .accounts
        .iter()
        .any(|entry| entry.pending.is_some() && !input.account_ids.contains(&entry.account_id))
    {
        return Err(invalid("执行中的账号请先关闭计划，等待结果后再移除"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut accounts = Vec::new();
    for id in input.account_ids {
        if !ids.insert(id.clone()) {
            return Err(invalid("账号不能重复"));
        }
        let account = available.iter().find(|a| a.account_id == id);
        if input.enabled && account.is_none() {
            return Err(invalid("所选账号已停用或不存在"));
        }
        let mut entry = old
            .accounts
            .iter()
            .find(|e| e.account_id == id)
            .cloned()
            .unwrap_or(Entry {
                account_id: id.clone(),
                name: account.map_or_else(|| id.clone(), |account| account.name.clone()),
                next_at,
                status: "waiting".into(),
                checked_at: None,
                message: String::new(),
                pending: None,
                run_id: None,
                failures: 0,
            });
        if input.resume_accounts.contains(&id)
            || !old.enabled
            || old.interval_minutes != input.interval_minutes
            || old.timing != input.timing
            || old.detection != input.detection
            || old.model != input.model.trim()
        {
            entry.next_at = next_at;
            // Only explicit resume clears a failed account.
            if entry.status == "paused"
                && entry.pending.is_none()
                && input.resume_accounts.contains(&id)
            {
                entry.status = "waiting".into();
                entry.message.clear();
                entry.failures = 0;
            }
        }
        if !input.enabled && old.active_account.as_ref() == Some(&id) {
            entry.status = "paused".into();
            entry.pending = None;
            entry.message = "计划已关闭，本轮进度已保留；勾选恢复后继续".into();
        }
        accounts.push(entry);
    }
    let plan = Schedule {
        enabled: input.enabled,
        model: input.model.trim().into(),
        interval_minutes: input.interval_minutes,
        timing: input.timing,
        accounts,
        detection: input.detection,
        active_account: if input.enabled {
            old.active_account
        } else {
            None
        },
    };
    write(host, &plan, version).await?;
    Ok(plan)
}
pub(crate) async fn tick(service: &Service, host: &HostClient) -> Result<(), PluginFault> {
    let Ok(_execution) = service.execution.try_acquire() else {
        return Ok(());
    };
    if read(host).await?.0.detection.is_full() {
        return detection::tick_reserved(service, host).await;
    }
    let (id, operation, model) = {
        let _lock = service.state.lock().await;
        let (mut plan, version) = read(host).await?;
        // A previous invocation ended without committing a result. Never infer it was not sent.
        if plan.accounts.iter().any(|e| e.pending.is_some()) {
            for entry in &mut plan.accounts {
                if entry.pending.take().is_some() {
                    entry.status = "paused".into();
                    entry.message = "上次调用结果未知，已暂停；核对后手动恢复".into();
                }
            }
            write(host, &plan, version).await?;
            return Ok(());
        }
        if !plan.enabled {
            return Ok(());
        }
        let now = now_ms();
        if !plan
            .accounts
            .iter()
            .any(|entry| entry.status != "paused" && entry.next_at <= now)
        {
            return Ok(());
        }
        // Calculate from the current instant, never replay a backlog of missed occurrences.
        let next_at = plan
            .timing
            .prepare(plan.interval_minutes)?
            .next_after(now)?;
        let Some(entry) = plan
            .accounts
            .iter_mut()
            .filter(|e| e.status != "paused" && e.next_at <= now)
            .min_by_key(|e| e.next_at)
        else {
            return Ok(());
        };
        let operation = crate::now_nanos().to_string();
        entry.pending = Some(operation.clone());
        entry.status = "running".into();
        entry.checked_at = Some(now_ms());
        entry.next_at = next_at;
        let id = entry.account_id.clone();
        write(host, &plan, version).await?;
        (id, operation, plan.model)
    };
    let result = async {
        // Diagnostics may target scheduling-disabled accounts; never switch identities.
        let account = host::runtime_account(host, &id).await?;
        if account.provider_id != "openai" {
            return Err(invalid("账号已删除或平台发生变化"));
        }
        host.call(
            "host.account.probe",
            json!({"account_id":id,"model":model}),
            serde_json::to_vec(
                &json!({"model":model,"input":"Reply with OK only.","max_output_tokens":32}),
            )
            .map_err(|_| invalid("探测编码失败"))?,
        )
        .await
        .map_err(|e| e.into_plugin_fault())
    }
    .await;
    let _lock = service.state.lock().await;
    let (mut plan, version) = read(host).await?;
    let Some(entry) = plan
        .accounts
        .iter_mut()
        .find(|e| e.account_id == id && e.pending.as_deref() == Some(&operation))
    else {
        return Ok(());
    };
    entry.pending = None;
    match result {
        Ok(reply)
            if reply.result["complete"] == true
                && reply.result["text"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty()) =>
        {
            entry.status = "ok".into();
            entry.failures = 0;
            entry.message = "短请求成功；未发现明确错误，额度窗口以平台返回为准".into();
        }
        Ok(_) => {
            entry.failures = entry.failures.saturating_add(1);
            entry.status = if entry.failures < 3 {
                "waiting"
            } else {
                "paused"
            }
            .into();
            if entry.failures < 3 {
                entry.next_at = now_ms() + 30_000;
            }
            entry.message = "收到空回答或截断结果，已暂停；不能据此判断风控".into();
        }
        Err(error) => {
            entry.failures = entry.failures.saturating_add(1);
            entry.status = if entry.failures < 3 {
                "waiting"
            } else {
                "paused"
            }
            .into();
            if entry.failures < 3 {
                entry.next_at = now_ms() + 30_000;
            }
            entry.message = classify(&error).into();
        }
    }
    if entry.failures > 0 && entry.failures < 3 {
        entry.message = format!("短探测失败 {}/3，下轮重试", entry.failures);
    }
    write(host, &plan, version).await
}
fn classify(error: &PluginFault) -> &'static str {
    let code = error.details.as_ref().and_then(|d| d["code"].as_str());
    if matches!(
        code,
        Some("account_deactivated" | "account_suspended" | "account_banned")
    ) {
        return "上游明确提示账号已停用或封禁，已暂停";
    }
    match error.http_status {
        Some(401) => "鉴权失败，可能需要重新授权，已暂停",
        Some(403) => "访问被拒绝，可能涉及权限、区域或风控，需人工核对；已暂停",
        Some(429) => "上游限流或额度不足，已暂停；不能据此判断封禁",
        Some(500..=599) => "上游服务异常，已暂停；不能据此判断风控",
        _ => "检测失败或超时，已暂停；不能据此判断风控",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_migration_keeps_timing_and_does_not_enable_full_detection() {
        let legacy = json!({"enabled":true,"model":"gpt-6-astra","interval_minutes":360,"timing":{"mode":"cron","cron":"0 9 * * *","daily_time":"09:00","timezone":"Asia/Shanghai"},"accounts":[{"account_id":"a","name":"a","next_at":123,"status":"paused","checked_at":null,"message":"old","pending":"uncertain"}]});
        let request = serde_json::from_value(json!({"namespace":"schedule","from_schema_version":2,"to_schema_version":3,"records":[{"key":"settings","version":7,"value":legacy}]})).unwrap();
        let result = serde_json::to_value(migrate(request).unwrap()).unwrap();
        let value = &result["changes"][0]["value"];
        assert_eq!(value["timing"], legacy["timing"]);
        assert_eq!(value["accounts"], legacy["accounts"]);
        assert_eq!(value["detection"]["method"], "short");
        assert!(value["active_account"].is_null());
    }

    #[test]
    fn run_migration_keeps_frozen_historical_records() {
        let request = serde_json::from_value(json!({"namespace":"runs","from_schema_version":1,"to_schema_version":2,"records":[{"key":"0","version":7,"value":{"id":"0-1","plan":{"frozen":true}}}]})).unwrap();
        let result = serde_json::to_value(migrate(request).unwrap()).unwrap();
        assert_eq!(result["changes"], json!([{"key":"0","action":"keep"}]));
    }
}
