use crate::invalid;
use gateway_plugin_sdk::{ErrorCode, PluginFault};
use serde_json::Value;
use std::{ffi::OsString, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

const LIMIT: usize = 2 * 1024 * 1024;
fn fault(message: impl Into<String>) -> PluginFault {
    PluginFault::new(ErrorCode::Fault, message)
}
struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
pub struct Engine {
    command: Vec<OsString>,
    worker: Mutex<Option<Worker>>,
    directory: Option<PathBuf>,
}

impl Engine {
    pub fn command(command: Vec<OsString>) -> Self {
        Self {
            command,
            worker: Mutex::new(None),
            directory: None,
        }
    }

    #[cfg(feature = "bundled-engine")]
    pub fn bundled() -> std::io::Result<Self> {
        use std::{
            io::Write,
            os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        };
        // 宿主 cwd 是本次插件实例的私有包目录，退出后由宿主回收。
        // /tmp 在正式容器中为 noexec；引擎及其动态库都应位于插件执行目录。
        let directory = std::env::current_dir()?.join(format!(
            ".model-health-{}-{}",
            std::process::id(),
            crate::now_nanos()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let path = directory.join("meow-engine");
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&path)?;
            file.write_all(include_bytes!(env!("MEOW_ENGINE_BINARY")))?;
            file.sync_all()
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_dir_all(&directory);
            return Err(error);
        }
        let mut engine = Self::command(vec![path.into_os_string()]);
        engine.directory = Some(directory);
        Ok(engine)
    }

    pub async fn query(&self, request: Value) -> Result<Value, PluginFault> {
        let mut slot = self.worker.lock().await;
        if slot.is_none() {
            let program = self
                .command
                .first()
                .ok_or_else(|| fault("检测引擎命令为空"))?;
            let mut command = Command::new(program);
            command.args(&self.command[1..]);
            if let Some(directory) = &self.directory {
                // PyInstaller onefile 也会解压动态库，不能再使用系统 /tmp。
                command.env("TMPDIR", directory);
            }
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .kill_on_drop(true);
            // A concurrent fork can briefly inherit a just-closed executable's
            // write descriptor before exec closes it. Retry only ETXTBSY before
            // a child exists; never replay an exchange or an inference request.
            let mut retries = 0;
            let mut child = loop {
                match command.spawn() {
                    Ok(child) => break child,
                    Err(error)
                        if cfg!(target_os = "linux")
                            && error.raw_os_error() == Some(26)
                            && retries < 5 =>
                    {
                        retries += 1;
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(error) => return Err(fault(format!("检测引擎启动失败：{error}"))),
                }
            };
            let input = child
                .stdin
                .take()
                .ok_or_else(|| fault("检测引擎没有输入管道"))?;
            let output = BufReader::new(
                child
                    .stdout
                    .take()
                    .ok_or_else(|| fault("检测引擎没有输出管道"))?,
            );
            *slot = Some(Worker {
                child,
                input,
                output,
            });
        }
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            slot.as_mut().unwrap().exchange(request),
        )
        .await;
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                *slot = None;
                Err(error)
            }
            Err(_) => {
                *slot = None;
                Err(PluginFault::new(ErrorCode::Timeout, "检测引擎响应超时"))
            }
        }
    }
}

impl Worker {
    async fn exchange(&mut self, request: Value) -> Result<Value, PluginFault> {
        let mut bytes = serde_json::to_vec(&request).map_err(|_| invalid("检测输入编码失败"))?;
        if bytes.len() >= LIMIT {
            return Err(invalid("检测输入超过大小限制"));
        }
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|e| fault(format!("检测引擎写入失败：{e}")))?;
        self.input
            .flush()
            .await
            .map_err(|e| fault(format!("检测引擎写入失败：{e}")))?;
        let mut output = Vec::new();
        (&mut self.output)
            .take((LIMIT + 1) as u64)
            .read_until(b'\n', &mut output)
            .await
            .map_err(|e| fault(format!("检测引擎读取失败：{e}")))?;
        if output.len() > LIMIT || output.last() != Some(&b'\n') {
            return Err(fault("检测引擎返回超限或中断的响应"));
        }
        let reply: Value =
            serde_json::from_slice(&output).map_err(|_| fault("检测引擎返回了无效 JSON"))?;
        if reply["ok"] != true {
            return Err(invalid(format!(
                "检测引擎：{}",
                reply["error"].as_str().unwrap_or("invalid_response")
            )));
        }
        Ok(reply["data"].clone())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(path) = &self.directory {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
