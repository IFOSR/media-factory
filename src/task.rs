//! 任务元数据与流式事件管理。
//!
//! - `TaskMeta`：任务状态（写入 output/<id>/task.json）
//! - `Event`：流式事件（SSE 广播）
//! - `TaskEvents`：各步骤发事件/更新状态的入口（CLI 模式下为 no-op）

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// 流水线步骤
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Step {
    Rewrite,
    Image,
    Podcast,
    Video,
}

impl Step {
    pub fn as_str(&self) -> &'static str {
        match self {
            Step::Rewrite => "rewrite",
            Step::Image => "image",
            Step::Podcast => "podcast",
            Step::Video => "video",
        }
    }
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Step> {
        match s {
            "rewrite" => Some(Step::Rewrite),
            "image" => Some(Step::Image),
            "podcast" => Some(Step::Podcast),
            "video" => Some(Step::Video),
            _ => None,
        }
    }
    pub fn all() -> [Step; 4] {
        [Step::Rewrite, Step::Image, Step::Podcast, Step::Video]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl StepStatus {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Running => "running",
            StepStatus::Done => "done",
            StepStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMeta {
    pub id: String,
    pub status: String, // pending / running / done / failed
    pub current_step: Option<String>,
    pub steps: HashMap<String, StepStatus>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl TaskMeta {
    pub fn new(id: &str) -> Self {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let mut steps = HashMap::new();
        for s in Step::all() {
            steps.insert(s.as_str().to_string(), StepStatus::Pending);
        }
        Self {
            id: id.to_string(),
            status: "running".to_string(),
            current_step: None,
            steps,
            created_at: now.clone(),
            updated_at: now,
            error: None,
        }
    }

    fn touch(&mut self) {
        self.updated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    }

    pub fn set_step(&mut self, step: Step, status: StepStatus) {
        self.steps.insert(step.as_str().to_string(), status);
        if status == StepStatus::Running {
            self.current_step = Some(step.as_str().to_string());
        }
        self.touch();
    }

    pub fn finish(&mut self, failed: bool, error: Option<String>) {
        self.status = if failed { "failed" } else { "done" }.to_string();
        self.current_step = None;
        self.error = error;
        self.touch();
    }
}

/// 流式事件（SSE 广播给前端）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum Event {
    Step { step: String, status: String },
    /// 思考链路动作日志（流式展示后台处理细节）
    Log { step: String, text: String },
    Chunk { step: String, delta: String },
    Artifact { step: String, name: String, url: String },
    Progress { step: String, percent: f64 },
    Task { status: String, error: Option<String> },
}

/// 任务事件广播注册表（task_id -> (sender, keepalive receiver)）
/// keepalive receiver 保持 channel 活跃，让 send 在无人订阅时也能缓冲（一次任务事件数远小于 buffer）。
type RegistryEntry = (broadcast::Sender<Event>, broadcast::Receiver<Event>);
static REGISTRY: OnceLock<Mutex<HashMap<String, RegistryEntry>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, RegistryEntry>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn create_channel(task_id: &str) -> broadcast::Sender<Event> {
    let (tx, rx) = broadcast::channel(1024);
    registry().lock().unwrap().insert(task_id.to_string(), (tx.clone(), rx));
    tx
}

pub fn subscribe(task_id: &str) -> Option<broadcast::Receiver<Event>> {
    registry().lock().unwrap().get(task_id).map(|(tx, _)| tx.subscribe())
}

#[allow(dead_code)]
pub fn drop_channel(task_id: &str) {
    registry().lock().unwrap().remove(task_id);
}

/// 事件发送 + 状态落盘入口（各步骤注入使用）
#[derive(Clone)]
pub struct TaskEvents {
    task_id: String,
    output_root: PathBuf,
    sender: Option<broadcast::Sender<Event>>,
}

impl TaskEvents {
    /// CLI 模式：不发事件，但会写 task.json
    pub fn local(output_root: &Path, task_id: &str) -> Self {
        Self {
            task_id: task_id.to_string(),
            output_root: output_root.to_path_buf(),
            sender: None,
        }
    }

    /// Web 模式：发事件 + 写 task.json
    pub fn streaming(output_root: &Path, task_id: &str) -> Self {
        let sender = create_channel(task_id);
        Self {
            task_id: task_id.to_string(),
            output_root: output_root.to_path_buf(),
            sender: Some(sender),
        }
    }

    fn emit(&self, ev: Event) {
        if let Some(s) = &self.sender {
            let _ = s.send(ev);
        }
    }

    fn load_meta(&self) -> TaskMeta {
        let p = self.output_root.join(&self.task_id).join("task.json");
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| TaskMeta::new(&self.task_id))
    }

    fn save_meta(&self, meta: &TaskMeta) {
        let dir = self.output_root.join(&self.task_id);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("task.json"), serde_json::to_string_pretty(meta).unwrap_or_default());
    }

    pub fn init(&self) {
        self.save_meta(&TaskMeta::new(&self.task_id));
        self.emit(Event::Task { status: "running".into(), error: None });
    }

    pub fn step_running(&self, step: Step) {
        let mut m = self.load_meta();
        m.set_step(step, StepStatus::Running);
        self.save_meta(&m);
        self.emit(Event::Step { step: step.as_str().into(), status: "running".into() });
    }

    pub fn step_done(&self, step: Step) {
        let mut m = self.load_meta();
        m.set_step(step, StepStatus::Done);
        self.save_meta(&m);
        self.emit(Event::Step { step: step.as_str().into(), status: "done".into() });
    }

    pub fn step_failed(&self, step: Step, err: &str) {
        let mut m = self.load_meta();
        m.set_step(step, StepStatus::Failed);
        self.save_meta(&m);
        self.emit(Event::Step { step: step.as_str().into(), status: "failed".into() });
        let _ = err;
    }

    #[allow(dead_code)] // 预留：未来接 LLM token 级真流式
    pub fn chunk(&self, step: Step, delta: &str) {
        self.emit(Event::Chunk { step: step.as_str().into(), delta: delta.into() });
    }

    /// 发送思考链路动作日志；同时 println（CLI 终端与 web 的 serve.log 均可见）
    pub fn log(&self, step: Step, text: &str) {
        println!("  · {}", text);
        self.emit(Event::Log { step: step.as_str().into(), text: text.into() });
    }

    pub fn artifact(&self, step: Step, name: &str) {
        let url = format!("/api/files/{}/{}", self.task_id, name);
        self.emit(Event::Artifact { step: step.as_str().into(), name: name.into(), url });
    }

    #[allow(dead_code)]
    pub fn progress(&self, step: Step, percent: f64) {
        self.emit(Event::Progress { step: step.as_str().into(), percent });
    }

    pub fn task_done(&self) {
        let mut m = self.load_meta();
        m.finish(false, None);
        self.save_meta(&m);
        self.emit(Event::Task { status: "done".into(), error: None });
    }

    pub fn task_error(&self, err: &str) {
        let mut m = self.load_meta();
        m.finish(true, Some(err.to_string()));
        self.save_meta(&m);
        self.emit(Event::Task { status: "failed".into(), error: Some(err.to_string()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_event_serializes() {
        let ev = Event::Log { step: "rewrite".into(), text: "读取参考文案".into() };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(s, r#"{"type":"log","step":"rewrite","text":"读取参考文案"}"#);
    }

    #[test]
    fn log_local_mode_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        let ev = TaskEvents::local(dir.path(), "t1");
        ev.log(Step::Rewrite, "测试");
    }

    #[test]
    fn log_streaming_mode_broadcasts() {
        let dir = tempfile::tempdir().unwrap();
        let ev = TaskEvents::streaming(dir.path(), "t2");
        let mut rx = subscribe("t2").unwrap();
        ev.log(Step::Image, "提炼图像 prompt");
        let got = rx.try_recv().unwrap();
        match got {
            Event::Log { step, text } => {
                assert_eq!(step, "image");
                assert_eq!(text, "提炼图像 prompt");
            }
            _ => panic!("expected Log event"),
        }
    }
}
