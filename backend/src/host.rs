use crate::{
    domain::{AccountIdentity, Run},
    invalid,
};
use gateway_plugin_sdk::{
    PluginFault,
    call::{host::*, model::WirePayload},
    client::HostClient,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub async fn metadata<I: Serialize, O: DeserializeOwned>(
    host: &HostClient,
    method: &str,
    input: I,
) -> Result<O, PluginFault> {
    let reply = host
        .call(
            method,
            serde_json::to_value(input).map_err(|_| invalid("回调编码失败"))?,
            vec![],
        )
        .await
        .map_err(|e| e.into_plugin_fault())?;
    if !reply.payload.is_empty() {
        return Err(invalid("宿主元数据回调夹带了正文"));
    }
    serde_json::from_value(reply.result).map_err(|_| invalid("宿主回调响应格式错误"))
}
async fn payload<I: Serialize, O: DeserializeOwned>(
    host: &HostClient,
    method: &str,
    input: I,
) -> Result<O, PluginFault> {
    let reply = host
        .call(
            method,
            json!({}),
            serde_json::to_vec(&input).map_err(|_| invalid("回调编码失败"))?,
        )
        .await
        .map_err(|e| e.into_plugin_fault())?;
    serde_json::from_slice(&reply.payload).map_err(|_| invalid("宿主账号回调响应格式错误"))
}

pub async fn accounts(host: &HostClient) -> Result<Vec<AuthRuntimeAccount>, PluginFault> {
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let mut accounts = vec![];
    loop {
        let page: AuthListResult = payload(
            host,
            "host.auth.list",
            AuthListRequest {
                provider_id: Some("openai".into()),
                cursor,
                limit: 100,
            },
        )
        .await?;
        accounts.extend(
            page.accounts
                .into_iter()
                .filter(|a| a.enabled && a.provider_id == "openai"),
        );
        cursor = page.next_cursor;
        let Some(next) = &cursor else {
            return Ok(accounts);
        };
        if !seen.insert(next.clone()) || seen.len() > 100 {
            return Err(invalid("账号分页重复或超过 100 页"));
        }
    }
}
pub async fn keys(host: &HostClient) -> Result<Vec<ClientKey>, PluginFault> {
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let mut keys = vec![];
    loop {
        let page = host
            .list_keys(KeyListRequest { cursor, limit: 100 })
            .await?;
        keys.extend(page.keys.into_iter().filter(|k| k.enabled));
        cursor = page.next_cursor;
        let Some(next) = &cursor else {
            return Ok(keys);
        };
        if !seen.insert(next.clone()) || seen.len() > 100 {
            return Err(invalid("Key 分页重复或超过 100 页"));
        }
    }
}
pub async fn models(host: &HostClient, key: &str) -> Result<ModelListResult, PluginFault> {
    metadata(
        host,
        "host.models.list",
        ModelListRequest {
            client_key_id: key.into(),
            protocol: "openai".into(),
            client_version: concat!("model-health/", env!("CARGO_PKG_VERSION")).into(),
        },
    )
    .await
}
pub fn account_identity(account: &AuthRuntimeAccount) -> AccountIdentity {
    AccountIdentity {
        authentication_kind: account.authentication_kind.clone(),
        upstream_user_id: account.upstream_user_id.clone(),
        upstream_account_id: account.upstream_account_id.clone(),
    }
}

pub async fn runtime_account(
    host: &HostClient,
    id: &str,
) -> Result<AuthRuntimeAccount, PluginFault> {
    payload(
        host,
        "host.auth.get_runtime",
        AuthGetRequest {
            account_id: id.into(),
        },
    )
    .await
}
pub async fn validate_account(host: &HostClient, run: &mut Run) -> Result<(), PluginFault> {
    let account = runtime_account(host, &run.account_id).await?;
    if !account.enabled || account.provider_id != "openai" || account.account_id != run.account_id {
        return Err(invalid(
            "检测账号已停用或所属账号/平台发生变化，请检查账号状态",
        ));
    }
    let identity = account_identity(&account);
    let revision_changed = account.credential_revision != run.credential_revision;
    match &run.account_identity {
        Some(expected) if expected != &identity => {
            return Err(invalid(
                "账号身份已改变，请结束当前检测后新建，避免混合不同账号的样本",
            ));
        }
        None if revision_changed && !run.attempts.is_empty() => {
            // 旧记录没有上游身份快照，不能用当前身份反填已完成样本的归属。
            return Err(invalid(
                "这条旧检测缺少账号身份快照且凭据已更新，请结束后新建检测；历史结果保留",
            ));
        }
        _ => {}
    }
    let has_identity = [
        identity.upstream_user_id.as_deref(),
        identity.upstream_account_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|id| !id.is_empty());
    if revision_changed && !has_identity && !run.attempts.is_empty() {
        return Err(invalid(
            "该账号缺少稳定的上游身份且凭据已更新，请结束后新建检测",
        ));
    }
    // Cookie 回写、刷新退避和令牌轮换都会推进 revision；它不是账号身份版本。
    // 随同 pending/续测状态一起保存本次检查值，不冒充实际发送后的凭据版本。
    run.account_identity = Some(identity);
    run.credential_revision = account.credential_revision;
    Ok(())
}
pub async fn read(host: &HostClient, slot: usize) -> Result<Option<(Run, u64)>, PluginFault> {
    let reply: StateGetResult = metadata(
        host,
        "host.state.get",
        StateGetRequest {
            namespace: "runs".into(),
            key: format!("run-{slot}"),
        },
    )
    .await?;
    reply
        .record
        .map(|r| {
            serde_json::from_value(r.value)
                .map(|run| (run, r.version))
                .map_err(|_| invalid("检测状态格式错误"))
        })
        .transpose()
}
pub async fn write(host: &HostClient, run: &Run, version: Option<u64>) -> Result<(), PluginFault> {
    let _: StatePutResult = metadata(
        host,
        "host.state.put",
        StatePutRequest {
            namespace: "runs".into(),
            key: format!("run-{}", run.slot),
            value: serde_json::to_value(run).map_err(|_| invalid("状态编码失败"))?,
            expected_version: version,
        },
    )
    .await?;
    Ok(())
}

pub struct ExecutionFailure {
    pub request_id: Option<String>,
    pub fault: PluginFault,
}
impl From<PluginFault> for ExecutionFailure {
    fn from(fault: PluginFault) -> Self {
        Self {
            request_id: None,
            fault,
        }
    }
}

pub async fn execute(
    host: &HostClient,
    run: &Run,
    body: Value,
) -> Result<(String, String), ExecutionFailure> {
    if run.key_id.is_empty() {
        let result: Value = host
            .call(
                "host.account.probe",
                json!({"account_id":run.account_id,"model":run.plan["request_model"]}),
                serde_json::to_vec(&body).map_err(|_| invalid("检测请求编码失败"))?,
            )
            .await
            .map_err(|e| e.into_plugin_fault())?
            .result;
        let text = result["text"]
            .as_str()
            .ok_or_else(|| invalid("账号检测结果格式错误"))?;
        if result["complete"] != true || text.is_empty() {
            return Err(invalid("账号返回了截断、拒答或空回答，本次不计为有效样本").into());
        }
        let response = json!({"type":"response.completed","response":{"id":"account-diagnostic","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]}});
        return Ok((String::new(), format!("data: {response}\n\n")));
    }
    let request = ModelExecuteRequest {
        client_key_id: Some(run.key_id.clone()),
        model: run.plan["request_model"]
            .as_str()
            .ok_or_else(|| invalid("缺少模型"))?
            .into(),
        protocol: "openai".into(),
        operation: ModelOperation::Generate,
        provider: Some("openai".into()),
        account_id: Some(run.account_id.clone()),
        previous_response_id: None,
    };
    let reply = host
        .call(
            "host.model.execute_stream",
            serde_json::to_value(request).map_err(|_| invalid("模型元数据编码失败"))?,
            serde_json::to_vec(&body).map_err(|_| invalid("模型正文编码失败"))?,
        )
        .await
        .map_err(|e| e.into_plugin_fault())?;
    let result: ModelStreamResult =
        serde_json::from_value(reply.result).map_err(|_| invalid("模型响应缺少请求 ID"))?;
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        read_stream(host, &result.stream),
    )
    .await;
    match read {
        Ok(Ok(sse)) => Ok((result.request_id, sse)),
        other => {
            let mut message = match other {
                Ok(Err(e)) => e.message,
                _ => "单题达到 90 秒上限".into(),
            };
            let close = host
                .call(
                    "host.model.stream_close",
                    json!({"stream":result.stream}),
                    vec![],
                )
                .await;
            if let Err(error) = close {
                message.push_str(&format!(
                    "；关闭执行流：{}",
                    error.into_plugin_fault().message
                ));
            }
            Err(ExecutionFailure {
                request_id: Some(result.request_id),
                fault: invalid(message),
            })
        }
    }
}

async fn read_stream(host: &HostClient, stream: &str) -> Result<String, PluginFault> {
    let mut sse = String::new();
    loop {
        let reply = host
            .call(
                "host.model.stream_read",
                json!({"stream":stream,"maximum_bytes":262144}),
                vec![],
            )
            .await
            .map_err(|e| e.into_plugin_fault())?;
        let state: ModelStreamReadResult =
            serde_json::from_value(reply.result).map_err(|_| invalid("模型流状态格式错误"))?;
        let batch = if reply.payload.is_empty() && state.events == 0 {
            ModelEventBatch::default()
        } else {
            ModelEventBatch::decode(&reply.payload).map_err(|_| invalid("模型事件批次格式错误"))?
        };
        if batch.events.len() != state.events as usize {
            return Err(invalid("模型事件数量不一致"));
        }
        for event in batch.events {
            let Some(wire) = event.wire else {
                continue;
            };
            if wire.protocol != "openai" {
                return Err(invalid("模型返回了不匹配的协议"));
            }
            match wire.payload {
                WirePayload::Json { data, .. }
                | WirePayload::WebSocket {
                    data: Some(data), ..
                } => {
                    sse.push_str("data: ");
                    sse.push_str(&data.to_string());
                    sse.push_str("\n\n");
                }
                WirePayload::RawSse { frame } => {
                    sse.push_str(std::str::from_utf8(&frame).map_err(|_| invalid("SSE 编码错误"))?)
                }
                _ => return Err(invalid("模型未返回 Responses SSE；检测中止以免误评分")),
            }
            if sse.len() > 1024 * 1024 {
                return Err(invalid("检测响应超过 1 MiB"));
            }
        }
        if state.end {
            return Ok(sse);
        }
    }
}
