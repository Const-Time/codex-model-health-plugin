use super::{Entry, read, write};
use crate::{
    Service,
    domain::{Method, Run, Status},
    host, invalid, management, now_ms,
};
use gateway_plugin_sdk::{PluginFault, client::HostClient};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    #[default]
    Short,
    Meow,
    Modeltrace,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Detection {
    pub method: Kind,
    pub tier: String,
    pub claimed_model: String,
}
impl Default for Detection {
    fn default() -> Self {
        Self {
            method: Kind::Short,
            tier: "low".into(),
            claimed_model: String::new(),
        }
    }
}
impl Detection {
    pub fn new_detection() -> Self {
        Self {
            method: Kind::Meow,
            ..Self::default()
        }
    }
    pub fn is_full(&self) -> bool {
        self.method != Kind::Short
    }
    fn query(&self, account_id: &str, model: &str) -> management::Start {
        management::Start {
            method: if self.method == Kind::Modeltrace {
                Method::ModelTrace
            } else {
                Method::Meow
            },
            account_id: account_id.into(),
            key_id: String::new(),
            request_model: model.into(),
            claimed_model: if self.method == Kind::Modeltrace {
                "unified".into()
            } else {
                self.claimed_model.clone()
            },
            tier: if self.method == Kind::Modeltrace {
                "fixed".into()
            } else {
                self.tier.clone()
            },
        }
    }
    pub async fn validate(
        &self,
        service: &Service,
        host: &HostClient,
        model: &str,
        enabled: bool,
    ) -> Result<(), PluginFault> {
        if self.claimed_model.len() > 128 || self.tier.len() > 16 {
            return Err(invalid("检测配置无效"));
        }
        if enabled && self.is_full() {
            if !host::scheduled_detection_supported(host).await {
                return Err(invalid(
                    "请先升级配套主程序：当前宿主仅支持定时短探测，不支持完整模型检测",
                ));
            }
            let query = self.query("", model);
            service.engine.query(json!({"op":"prepare","method":query.method,"tier":query.tier,"claimed_model":query.claimed_model,"request_model":query.request_model})).await?;
        }
        Ok(())
    }
}

// Caller holds the state lock. Pause is durable even if the current question is still returning.
pub(super) async fn pause_active(
    _service: &Service,
    host: &HostClient,
    plan: &super::Schedule,
) -> Result<(), PluginFault> {
    for entry in plan.accounts.iter().filter(|e| e.status == "running") {
        if let Some(id) = entry.run_id.as_deref()
            && let Ok((mut run, version)) = management::find(host, id).await
            && !run.finished()
        {
            run.status = Status::Paused;
            host::write(host, &run, Some(version)).await?;
        }
    }
    Ok(())
}

async fn start_or_resume(
    service: &Service,
    host: &HostClient,
    entry: &Entry,
    detection: &Detection,
    model: &str,
) -> Result<Run, PluginFault> {
    let query = detection.query(&entry.account_id, model);
    {
        let _lock = service.state.lock().await;
        if let Some(id) = &entry.run_id
            && let Ok((mut run, version)) = management::find(host, id).await
            && run.scheduled
            && run.status == Status::Paused
            && run.attempts.len() < 324
            && run.plan["request_model"] == query.request_model
            && run.plan["claimed_model"] == query.claimed_model
            && run.plan["tier"] == query.tier
            && run.plan["method"] == json!(query.method)
        {
            host::validate_account(host, &mut run).await?;
            run.recover()?;
            host::write(host, &run, Some(version)).await?;
            return Ok(run);
        }
    }
    management::create_run(service, host, query, true).await
}

// Each selected account creates a run OR executes one question per tick.
// The batch lease excludes manual execution and overlapping reconciliation calls.
pub(super) async fn tick_reserved(
    service: &Service,
    host: &HostClient,
    account_id: &str,
) -> Result<(), PluginFault> {
    let (entry, detection, model, starting) =
        {
            let _lock = service.state.lock().await;
            let (mut plan, version) = read(host).await?;
            if !plan.enabled {
                return Ok(());
            }
            if plan
                .accounts
                .iter()
                .any(|e| e.account_id == account_id && e.status == "running")
            {
                let Some(entry) = plan
                    .accounts
                    .iter_mut()
                    .find(|e| e.account_id == account_id)
                else {
                    return Err(invalid("活动检测账号缺失"));
                };
                if entry.pending.is_some() || entry.run_id.is_none() {
                    entry.pending = None;
                    entry.status = "paused".into();
                    entry.message = "上次创建检测任务中断，请核对后手动恢复".into();

                    write(host, &plan, version).await?;
                    return Ok(());
                }
                (entry.clone(), plan.detection, plan.model, false)
            } else {
                let now = now_ms();
                let next = plan
                    .timing
                    .prepare(plan.interval_minutes)?
                    .next_after(now)?;
                let Some(entry) = plan.accounts.iter_mut().find(|e| {
                    e.account_id == account_id && e.status != "paused" && e.next_at <= now
                }) else {
                    return Ok(());
                };
                entry.status = "running".into();
                entry.pending = Some("creating-detection".into());
                entry.checked_at = Some(now);
                entry.next_at = next;
                let snapshot = entry.clone();

                write(host, &plan, version).await?;
                (snapshot, plan.detection, plan.model, true)
            }
        };
    if starting {
        let result = start_or_resume(service, host, &entry, &detection, &model).await;
        let _lock = service.state.lock().await;
        let (mut plan, version) = read(host).await?;
        if !plan.enabled
            || !plan
                .accounts
                .iter()
                .any(|e| e.account_id == entry.account_id && e.status == "running")
        {
            if let Ok(mut run) = result {
                let (_, v) = management::find(host, &run.id).await?;
                run.status = Status::Paused;
                host::write(host, &run, Some(v)).await?;
            }
            return Ok(());
        }
        let current = plan
            .accounts
            .iter_mut()
            .find(|e| e.account_id == entry.account_id)
            .ok_or_else(|| invalid("账号已移除"))?;
        current.pending = None;
        match result {
            Ok(run) => {
                current.run_id = Some(run.id.clone());
                current.failures = 0;
                let valid = run
                    .jobs()
                    .iter()
                    .filter_map(|job| job["job_id"].as_str())
                    .filter(|job| run.successful(job))
                    .count();
                current.message = format!("检测已就绪，{valid}/{} 个有效样本", run.jobs().len());
            }
            Err(_) => {
                current.failures = current.failures.saturating_add(1);
                current.status = if current.failures >= 3 {
                    "paused"
                } else {
                    "waiting"
                }
                .into();
                current.next_at = now_ms() + 30_000;
                current.message = format!(
                    "创建检测失败 {}/3{}",
                    current.failures,
                    if current.failures >= 3 {
                        "，已暂停"
                    } else {
                        "，下轮重试"
                    }
                );
            }
        }
        return write(host, &plan, version).await;
    }
    let id = entry
        .run_id
        .as_deref()
        .ok_or_else(|| invalid("检测任务缺失"))?;
    let result=async {
        {
            let _lock=service.state.lock().await;
            let (mut run,version)=management::find(host,id).await?;
            if run.inflight.is_some() {
                run.finish_attempt(json!({"status":"error","error":"interrupted_unknown_result","answer":"","request_id":null}))?;
                management::update_report(service,&mut run).await?;
                host::write(host,&run,Some(version)).await?;
                return Ok(run);
            }
            if run.status!=Status::Ready { return Ok(run); }
        }
        management::step_reserved(service,host,id).await
    }.await;
    let _lock = service.state.lock().await;
    let (mut plan, version) = read(host).await?;
    if !plan
        .accounts
        .iter()
        .any(|e| e.account_id == entry.account_id && e.status == "running")
    {
        return Ok(());
    }
    let next = plan
        .timing
        .prepare(plan.interval_minutes)?
        .next_after(now_ms())?;
    let current = plan
        .accounts
        .iter_mut()
        .find(|e| e.account_id == entry.account_id)
        .ok_or_else(|| invalid("账号已移除"))?;
    match result {
        Ok(run) => {
            current.failures = run.current_failures().min(3) as u8;
            let valid = run
                .jobs()
                .iter()
                .filter_map(|j| j["job_id"].as_str())
                .filter(|j| run.successful(j))
                .count();
            current.message = match run.status {
                Status::Completed => format!(
                    "检测完成：{}；{}/{} 个有效样本",
                    report_summary(&run),
                    valid,
                    run.jobs().len()
                ),
                Status::Paused | Status::Stopped => format!(
                    "检测已暂停；{}/{} 个有效样本，本题失败 {}/3，可查看报告和请求明细",
                    valid,
                    run.jobs().len(),
                    current.failures
                ),
                _ if current.failures > 0 => format!(
                    "本题失败 {}/3，下轮重试；{}/{} 个有效样本",
                    current.failures,
                    valid,
                    run.jobs().len()
                ),
                _ => format!("检测中：{}/{} 个有效样本", valid, run.jobs().len()),
            };
            match run.status {
                Status::Completed => {
                    current.status = "ok".into();
                    current.next_at = next;
                }
                Status::Paused | Status::Stopped => {
                    current.status = "paused".into();
                }
                _ => {}
            }
        }
        Err(_) => {
            current.failures = current.failures.saturating_add(1);
            current.message = format!(
                "检测步骤失败 {}/3{}",
                current.failures,
                if current.failures >= 3 {
                    "，已暂停"
                } else {
                    "，下轮重试"
                }
            );
            if current.failures >= 3 {
                current.status = "paused".into();

                if let Ok((mut run, v)) = management::find(host, id).await {
                    run.status = Status::Paused;
                    host::write(host, &run, Some(v)).await?;
                }
            }
        }
    }
    write(host, &plan, version).await
}

fn report_summary(run: &Run) -> String {
    let verdict = match run.report["verdict"].as_str() {
        Some("match") => "与基线匹配",
        Some("mismatch") => "与基线不匹配",
        Some("attributed") => "已生成模型归属结果",
        _ => "证据不足或未能归属",
    };
    let model = run.report["model"]
        .as_str()
        .or_else(|| run.report["analysis"]["prediction_name"].as_str())
        .unwrap_or("");
    format!("{verdict} {model}")
}
