use crate::{
    Service,
    domain::{Method, Run, SLOTS, Status, slot_from_id},
    host, invalid, now_ms, now_nanos, updates,
};
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::management::*,
    client::{HostClient, TypedCall, TypedReply},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn registration() -> ManagementRegistration {
    ManagementRegistration {
        routes: [
            ("GET", "api/bootstrap"),
            ("POST", "api/upstreams/check"),
            ("POST", "api/models"),
            ("GET", "api/runs"),
            ("POST", "api/runs/get"),
            ("POST", "api/runs/start"),
            ("POST", "api/runs/step"),
            ("POST", "api/runs/control"),
            ("GET", "api/schedule"),
            ("POST", "api/schedule"),
            ("POST", "api/schedule/preview"),
        ]
        .into_iter()
        .map(|(method, path)| ManagementRoute {
            method: method.into(),
            path: path.into(),
            request_content_types: if method == "POST" {
                vec!["application/json".into()]
            } else {
                vec![]
            },
            response_content_types: vec!["application/json".into()],
        })
        .collect(),
        resources: ["web/index.html", "web/app.js", "web/app.css"]
            .into_iter()
            .map(|path| ManagementResource {
                path: path.into(),
                public: false,
            })
            .collect(),
        pages: vec![ManagementPage {
            id: "model-health".into(),
            title: "账号模型体检".into(),
            description: Some("选择已有账号，使用 Meow 或 ModelTrace 检测模型行为指纹".into()),
            entry: "web/index.html".into(),
            icon: None,
        }],
        callbacks: vec![],
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Start {
    #[serde(default)]
    pub method: Method,
    pub account_id: String,
    #[serde(default)]
    pub key_id: String,
    pub request_model: String,
    pub claimed_model: String,
    pub tier: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    id: String,
    action: String,
}

fn input<T: serde::de::DeserializeOwned>(
    call: &TypedCall<ManagementRequest>,
) -> Result<T, PluginFault> {
    serde_json::from_slice(&call.payload).map_err(|_| invalid("请求参数格式错误"))
}
fn reply(value: Value) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    reply_status(200, value)
}
fn reply_status(status: u16, value: Value) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    Ok(TypedReply::new(ManagementResponse {
        status,
        content_type: "application/json".into(),
        headers: vec![],
    })
    .with_payload(serde_json::to_vec(&value).map_err(|_| invalid("响应编码失败"))?))
}
pub(crate) async fn find(host: &HostClient, id: &str) -> Result<(Run, u64), PluginFault> {
    let (run, version) = host::read(host, slot_from_id(id)?)
        .await?
        .ok_or_else(|| invalid("检测记录不存在"))?;
    if run.id != id {
        return Err(invalid("检测记录已超出 16 条历史保留窗口"));
    }
    Ok((run, version))
}
async fn histories(host: &HostClient) -> Result<Vec<(Run, u64)>, PluginFault> {
    let mut runs = Vec::new();
    for slot in 0..SLOTS {
        if let Some(run) = host::read(host, slot).await? {
            runs.push(run);
        }
    }
    runs.sort_by_key(|(r, _)| std::cmp::Reverse(r.created_at));
    Ok(runs)
}
pub(crate) async fn update_report(service: &Service, run: &mut Run) -> Result<(), PluginFault> {
    run.report = service
        .engine
        .query(json!({"op":"score","plan":run.plan,"results":run.attempts}))
        .await?;
    run.updated_at = now_ms();
    Ok(())
}

pub async fn handle(
    service: &Service,
    call: TypedCall<ManagementRequest>,
) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    let path = call.request.path.clone();
    let background = call
        .request
        .headers
        .iter()
        .any(|h| h.name == "x-cpr-background-version" && h.value == b"1");
    let active = call
        .request
        .headers
        .iter()
        .find(|h| h.name == "x-cpr-background-active")
        .and_then(|h| std::str::from_utf8(&h.value).ok())
        .map(str::to_owned);
    let resume = path == "api/runs/control"
        && serde_json::from_slice::<Value>(&call.payload)
            .ok()
            .is_some_and(|v| v["action"] == "resume");
    if path == "api/runs/start" || resume || path == "api/runs/step" {
        if !background {
            return reply_status(
                409,
                json!({"error": PluginFault::new(ErrorCode::Conflict, "宿主尚不支持后台检测，请先升级配套的 Codex Proxy 宿主")}),
            );
        }
        if active.is_some() && (path == "api/runs/start" || resume) {
            return reply_status(
                409,
                json!({"error": PluginFault::new(ErrorCode::Conflict, "已有后台检测正在执行，请先暂停或结束")}),
            );
        }
    }
    let call_host = call.host.clone();
    match dispatch(service, call).await {
        Ok(mut response) => {
            let mut value: Value =
                serde_json::from_slice(&response.payload).map_err(|_| invalid("响应格式错误"))?;
            if path == "api/bootstrap" {
                value["background_supported"] = json!(background);
            }
            let continues = (path == "api/runs/start" || resume || path == "api/runs/step")
                && value["run"]["status"] == "ready"
                && value["run"]["scheduled"] != true;
            if continues {
                let id = value["run"]["id"]
                    .as_str()
                    .ok_or_else(|| invalid("任务标识缺失"))?;
                response.result.headers.push(gateway_plugin_sdk::call::middleware::MiddlewareHeader {
                    name: "x-cpr-background-next".into(),
                    value: serde_json::to_vec(&json!({"id":id,"path":"api/runs/step","body":json!({"id":id}).to_string()})).map_err(|_| invalid("后台任务编码失败"))?,
                });
            }
            if value.get("run").is_some() {
                let scheduled_running = if value["run"]["scheduled"] == true
                    && matches!(value["run"]["status"].as_str(), Some("ready" | "running"))
                {
                    let schedule = crate::schedule::read(&call_host).await?.0;
                    schedule.enabled
                        && schedule.accounts.iter().any(|entry| {
                            entry
                                .run_id
                                .as_deref()
                                .is_some_and(|id| value["run"]["id"] == id)
                                && entry.status == "running"
                        })
                } else {
                    false
                };
                value["background_running"] = json!(
                    scheduled_running
                        || continues
                        || active.as_deref().is_some_and(|id| value["run"]["id"] == id)
                );
            }
            response.payload = serde_json::to_vec(&value).map_err(|_| invalid("响应编码失败"))?;
            Ok(response)
        }
        Err(error) => {
            let status = match error.code {
                ErrorCode::InvalidInput => 400,
                ErrorCode::Rejected | ErrorCode::PermissionDenied => 403,
                ErrorCode::Unsupported => 404,
                ErrorCode::Conflict => 409,
                ErrorCode::Capacity => 503,
                ErrorCode::Timeout => 504,
                ErrorCode::Upstream => 502,
                ErrorCode::Fault | ErrorCode::Cancelled | ErrorCode::Uncertain => 500,
            };
            // 应用错误走 HTTP 响应；协议 Error 会被宿主替换成泛化的“副作用”提示。
            // 不转发原始 details；保留有界原因和发送事实，页面不会自动重试。
            reply_status(
                status,
                json!({"error": {
                    "code": error.code,
                    "message": error.message.chars().take(512).collect::<String>(),
                    "send_state": error.send_state,
                }}),
            )
        }
    }
}

async fn dispatch(
    service: &Service,
    call: TypedCall<ManagementRequest>,
) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    if !call.request.query.is_empty() || call.payload.len() > 8192 {
        return Err(invalid("管理请求大小或查询参数不符合接口"));
    }
    match (call.request.method.as_str(), call.request.path.as_str()) {
        ("POST", "api/schedule/preview") => reply(crate::schedule::preview(input(&call)?)?),
        ("GET", "api/schedule") => reply(
            serde_json::to_value(crate::schedule::read(&call.host).await?.0)
                .map_err(|_| invalid("计划编码失败"))?,
        ),
        ("POST", "api/schedule") => reply(
            serde_json::to_value(crate::schedule::save(service, &call.host, input(&call)?).await?)
                .map_err(|_| invalid("计划编码失败"))?,
        ),
        ("GET", "api/bootstrap") => {
            let catalog = service.engine.query(json!({"op":"catalog"})).await?;
            let accounts = host::accounts(&call.host)
                .await?
                .into_iter()
                .map(|a| {
                    json!({"id":a.account_id,"name":a.name,"email":a.email,"provider":a.provider_id,"plan":a.plan_type,"enabled":a.enabled})
                })
                .collect::<Vec<_>>();
            reply(
                json!({"catalog":catalog,"sources":updates::sources()?,"accounts":accounts,"keys":[],"account_probe":true,"scheduled_detection_supported":host::scheduled_detection_supported(&call.host).await,"maximum_concurrent_accounts":host::diagnostic_concurrency(&call.host).await}),
            )
        }
        ("POST", "api/upstreams/check") => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Check {
                method: Method,
            }
            let query: Check = input(&call)?;
            reply(json!({"check":service.updates.check(&call.host, query.method).await?}))
        }
        ("POST", "api/models") => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct ModelInput {
                key_id: String,
            }
            let query: ModelInput = input(&call)?;
            reply(
                serde_json::to_value(host::models(&call.host, &query.key_id).await?)
                    .map_err(|_| invalid("模型列表编码失败"))?,
            )
        }
        ("GET", "api/runs") => reply(
            json!({"runs":histories(&call.host).await?.iter().map(|(r,_)|r.summary()).collect::<Vec<_>>()}),
        ),
        ("POST", "api/runs/get") => {
            let query: Identity = input(&call)?;
            let (run, _) = find(&call.host, &query.id).await?;
            reply(json!({"run":run,"busy":service.execution.available_permits()==0}))
        }
        ("POST", "api/runs/start") => start(service, &call.host, input(&call)?).await,
        ("POST", "api/runs/control") => control(service, &call.host, input(&call)?).await,
        ("POST", "api/runs/step") => {
            let query: Identity = input(&call)?;
            step(service, &call.host, &query.id).await
        }
        _ => Err(PluginFault::new(
            ErrorCode::Unsupported,
            "未注册的插件管理接口",
        )),
    }
}

async fn start(
    service: &Service,
    host: &HostClient,
    query: Start,
) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    reply(json!({"run":create_run(service, host, query, false).await?}))
}
pub(crate) async fn create_run(
    service: &Service,
    host: &HostClient,
    query: Start,
    scheduled: bool,
) -> Result<Run, PluginFault> {
    let account = host::accounts(host)
        .await?
        .into_iter()
        .find(|a| a.account_id == query.account_id)
        .ok_or_else(|| invalid("请选择已启用的 OpenAI 账号"))?;
    // Keep already-open older pages and historical runs on their original billing authority.
    let legacy_key = if query.key_id.is_empty() {
        None
    } else {
        let key = host::keys(host)
            .await?
            .into_iter()
            .find(|k| k.id == query.key_id)
            .ok_or_else(|| invalid("原检测 Key 已不可用"))?;
        if !host::models(host, &query.key_id)
            .await?
            .models
            .contains(&query.request_model)
        {
            return Err(invalid("原检测 Key 不支持所选模型"));
        }
        Some(key)
    };
    if query.request_model.is_empty()
        || query.request_model.len() > 128
        || query.request_model.chars().any(char::is_control)
    {
        return Err(invalid("请求模型无效"));
    }
    let plan=service.engine.query(json!({"op":"prepare","method":query.method,"tier":query.tier,"claimed_model":query.claimed_model,"request_model":query.request_model})).await?;
    let _lock = service.state.lock().await;
    let history = histories(host).await?;
    let schedule = crate::schedule::read(host).await?.0;
    let active_runs: Vec<_> = schedule
        .accounts
        .iter()
        .filter(|entry| entry.status == "running")
        .filter_map(|entry| entry.run_id.as_deref())
        .collect();
    let slot = (0..SLOTS)
        .find(|s| !history.iter().any(|(r, _)| r.slot == *s))
        .or_else(|| {
            history
                .iter()
                .rev()
                .find(|(r, _)| {
                    r.finished()
                        || (r.scheduled
                            && r.status == Status::Paused
                            && r.inflight.is_none()
                            && !active_runs.contains(&r.id.as_str()))
                })
                .map(|(r, _)| r.slot)
        })
        .ok_or_else(|| invalid("已有 16 个未结束任务，请先结束一个任务"))?;
    let version = history
        .iter()
        .find(|(r, _)| r.slot == slot)
        .map(|(_, v)| *v);
    let account_identity = host::account_identity(&account);
    let mut run = Run {
        id: format!("{slot}-{}", now_nanos()),
        slot,
        created_at: now_ms(),
        updated_at: now_ms(),
        account_id: account.account_id,
        account_name: account_name_for_display(account.email.as_deref(), &account.name),
        credential_revision: account.credential_revision,
        account_identity: Some(account_identity),
        key_id: legacy_key
            .as_ref()
            .map_or_else(String::new, |key| key.id.clone()),
        key_name: legacy_key.map_or_else(|| "管理员账号检测".into(), |key| key.name),
        plan,
        status: Status::Ready,
        attempts: vec![],
        inflight: None,
        report: Value::Null,
        scheduled,
        retry_from: 0,
    };
    update_report(service, &mut run).await?;
    host::write(host, &run, version).await?;
    Ok(run)
}

async fn control(
    service: &Service,
    host: &HostClient,
    query: Control,
) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    if !["pause", "resume", "stop"].contains(&query.action.as_str()) {
        return Err(invalid("未知的任务控制操作"));
    }
    // 只有明确续测才修复上次中断；并发中的真实请求仍持有执行租约。
    let _execution = if query.action == "resume" {
        Some(
            service
                .execution
                .try_acquire()
                .map_err(|_| invalid("当前题目尚在完成，请稍后刷新"))?,
        )
    } else {
        None
    };
    let _lock = service.state.lock().await;
    let (mut run, version) = find(host, &query.id).await?;
    if run.scheduled && query.action == "resume" {
        return Err(invalid("请在定时计划中勾选恢复此账号并保存"));
    }
    if run.finished() {
        return Err(invalid("任务已结束，历史报告保持不变"));
    }
    match query.action.as_str() {
        "pause" => run.status = Status::Paused,
        "stop" => {
            run.status = Status::Stopped;
            if service.execution.available_permits() > 0 && run.inflight.is_some() {
                run.recover()?;
                run.status = Status::Stopped;
            }
        }
        "resume" => {
            host::validate_account(host, &mut run).await?;
            run.recover()?;
        }
        _ => unreachable!(),
    }
    update_report(service, &mut run).await?;
    host::write(host, &run, Some(version)).await?;
    reply(json!({"run":run}))
}

async fn step(
    service: &Service,
    host: &HostClient,
    id: &str,
) -> Result<TypedReply<ManagementResponse>, PluginFault> {
    let _execution = service
        .execution
        .try_acquire()
        .map_err(|_| invalid("另一个检测请求正在执行，请等待该题完成"))?;
    let (run, _) = find(host, id).await?;
    if run.scheduled {
        return Err(invalid("定时检测由后台计划逐题执行"));
    }
    reply(json!({"run":step_reserved(service,host,id).await?}))
}
pub(crate) async fn step_reserved(
    service: &Service,
    host: &HostClient,
    id: &str,
) -> Result<Run, PluginFault> {
    let (snapshot, job, body) = {
        let _lock = service.state.lock().await;
        let (mut run, version) = find(host, id).await?;
        if run.scheduled {
            let plan = crate::schedule::read(host).await?.0;
            if !plan.enabled
                || !plan
                    .accounts
                    .iter()
                    .any(|entry| entry.run_id.as_deref() == Some(id) && entry.status == "running")
            {
                return Err(invalid("定时计划已暂停或切换"));
            }
        }
        host::validate_account(host, &mut run).await?;
        let job = run.next_job().ok_or_else(|| invalid("检测计划已完成"))?;
        let body = service
            .engine
            .query(json!({"op":"payload","plan":run.plan,"job_id":job}))
            .await?;
        run.reserve(&job)?;
        run.updated_at = now_ms();
        // 发送前写入 pending；进程或页面中断不会恢复成免费重试。
        host::write(host, &run, Some(version)).await?;
        (run, job, body)
    };
    let result = match host::execute(host, &snapshot, body).await {
        Ok((request_id, sse)) => match service
            .engine
            .query(json!({"op":"evaluate","plan":snapshot.plan,"job_id":job,"sse":sse}))
            .await
        {
            Ok(mut sample) => {
                sample["request_id"] = json!(request_id);
                sample
            }
            Err(error) => {
                json!({"status":"error","error":excerpt(&error.message),"request_id":request_id})
            }
        },
        Err(error) => {
            json!({"status":"error","error":excerpt(&error.fault.message),"request_id":error.request_id})
        }
    };
    let _lock = service.state.lock().await;
    let (mut latest, version) = find(host, id).await?;
    latest.finish_attempt(result)?;
    update_report(service, &mut latest).await?;
    host::write(host, &latest, Some(version)).await?;
    Ok(latest)
}

fn excerpt(message: &str) -> String {
    let mut end = message.len().min(512);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
}

fn account_name_for_display(email: Option<&str>, name: &str) -> String {
    email
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let trimmed_name = name.trim();
            (!trimmed_name.is_empty()).then_some(trimmed_name)
        })
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::account_name_for_display;

    #[test]
    fn account_name_prefers_trimmed_email() {
        assert_eq!(
            account_name_for_display(Some("  first@example.com  "), "OPENAI账号"),
            "first@example.com"
        );
    }

    #[test]
    fn account_name_falls_back_to_trimmed_name() {
        assert_eq!(
            account_name_for_display(Some("   "), "  OPENAI账号  "),
            "OPENAI账号"
        );
    }

    #[test]
    fn account_name_allows_empty_fallback() {
        assert_eq!(account_name_for_display(None, "   "), "");
    }
}
