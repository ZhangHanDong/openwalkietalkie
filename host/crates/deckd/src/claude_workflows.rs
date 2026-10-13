//! Claude's public background-workflow metadata. The parent only records launches;
//! journals and worker usage continue changing after the parent turn has ended.
//! Never execute scripts, follow provider-supplied paths, or retain worker content.
use deck_model::workflow::{ActivityStatus, Run, MAX_RUNS};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const CHUNK: u64 = 4 * 1024 * 1024;
const MAX_WORKERS: usize = 128;

#[derive(Default)]
struct Cursor {
    offset: u64,
    pending: Vec<u8>,
    dropping: bool,
}
impl Cursor {
    // Incremental, bounded reads, including partial writes and oversized records.
    fn read<T: serde::de::DeserializeOwned>(&mut self, path: &Path) -> Vec<T> {
        let Ok(mut file) = File::open(path) else {
            return Vec::new();
        };
        let Ok(meta) = file.metadata() else {
            return Vec::new();
        };
        if !meta.is_file() {
            return Vec::new();
        }
        if meta.len() < self.offset {
            *self = Self::default();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        if file.take(CHUNK).read_to_end(&mut bytes).is_err() {
            return Vec::new();
        }
        self.offset += bytes.len() as u64;
        let mut rows = Vec::new();
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            let end = part.last() == Some(&b'\n');
            if self.pending.len() + part.len() > CHUNK as usize {
                self.pending.clear();
                self.dropping = true;
            }
            if !self.dropping {
                self.pending.extend_from_slice(part);
            }
            if end {
                if !self.dropping {
                    if let Ok(row) = serde_json::from_slice(&self.pending) {
                        rows.push(row);
                    }
                }
                self.pending.clear();
                self.dropping = false;
            }
        }
        rows
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParentRow {
    session_id: Option<String>,
    #[serde(default)]
    is_sidechain: bool,
    timestamp: Option<String>,
    tool_use_result: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Launch {
    status: String,
    task_id: String,
    workflow_name: String,
    run_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalRow {
    #[serde(rename = "type")]
    kind: String,
    key: Option<String>,
    agent_id: Option<String>,
}
#[derive(Default, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}
#[derive(Deserialize)]
struct WorkerRow {
    timestamp: Option<String>,
    message: Option<WorkerMessage>,
}
#[derive(Deserialize)]
struct WorkerMessage {
    id: Option<String>,
    usage: Option<Usage>,
}
#[derive(Default)]
struct Worker {
    id: String,
    done: bool,
    first_ms: Option<u64>,
    cursor: Cursor,
    input: u64,
    outputs: BTreeMap<String, u64>,
    retired_output: u64,
}
impl Worker {
    fn update(&mut self, row: WorkerRow) {
        if let Some(time) = row.timestamp.as_deref().and_then(timestamp_ms) {
            self.first_ms = Some(self.first_ms.map_or(time, |old| old.min(time)));
        }
        let Some(WorkerMessage {
            id: Some(id),
            usage: Some(usage),
        }) = row.message
        else {
            return;
        };
        self.input = usage
            .input_tokens
            .saturating_add(usage.cache_creation_input_tokens)
            .saturating_add(usage.cache_read_input_tokens);
        // Claude writes several blocks with the same message id/usage.
        self.outputs
            .entry(id)
            .and_modify(|n| *n = (*n).max(usage.output_tokens))
            .or_insert(usage.output_tokens);
        if self.outputs.len() > 4096 {
            if let Some((_, value)) = self.outputs.pop_first() {
                self.retired_output = self.retired_output.saturating_add(value);
            }
        }
    }
    fn tokens(&self) -> u64 {
        self.outputs
            .values()
            .fold(self.input.saturating_add(self.retired_output), |n, v| {
                n.saturating_add(*v)
            })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinalRun {
    run_id: String,
    task_id: String,
    status: String,
    duration_ms: u64,
    agent_count: u32,
    total_tokens: u64,
    #[serde(default)]
    workflow_progress: Vec<Progress>,
}
#[derive(Deserialize)]
struct Progress {
    #[serde(rename = "type")]
    kind: String,
    index: Option<u32>,
    state: Option<String>,
}
struct Tracked {
    run: Run,
    task: String,
    started: u64,
    journal: Cursor,
    workers: BTreeMap<String, Worker>,
    final_seen: bool,
}

pub(crate) struct Tracker {
    parent: PathBuf,
    root: PathBuf,
    session: String,
    cursor: Cursor,
    runs: BTreeMap<String, Tracked>,
}
fn timestamp_ms(s: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .and_then(|t| u64::try_from(t.timestamp_millis()).ok())
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}
fn safe_file(root: &Path, path: &Path) -> Option<PathBuf> {
    let expected = root.parent()?.canonicalize().ok()?.join(root.file_name()?);
    let root = root.canonicalize().ok()?;
    if root != expected {
        return None;
    }
    let path = path.canonicalize().ok()?;
    (path.starts_with(root) && path.is_file()).then_some(path)
}
impl Tracker {
    pub(crate) fn new(parent: &Path, session: &str) -> Self {
        Self {
            parent: parent.into(),
            root: parent.with_extension(""),
            session: session.into(),
            cursor: Cursor::default(),
            runs: BTreeMap::new(),
        }
    }
    pub(crate) fn poll(&mut self, now: u64) -> Vec<Run> {
        for row in self.cursor.read::<ParentRow>(&self.parent) {
            if row.is_sidechain
                || row
                    .session_id
                    .as_deref()
                    .is_some_and(|id| id != self.session)
            {
                continue;
            }
            let Some(launch) = row
                .tool_use_result
                .and_then(|v| serde_json::from_value::<Launch>(v).ok())
            else {
                continue;
            };
            if launch.status != "async_launched"
                || !identifier(&launch.run_id)
                || !identifier(&launch.task_id)
            {
                continue;
            }
            let Some(started) = row.timestamp.as_deref().and_then(timestamp_ms) else {
                continue;
            };
            if self
                .runs
                .get(&launch.run_id)
                .is_some_and(|r| r.task == launch.task_id)
            {
                continue;
            }
            self.runs.insert(
                launch.run_id.clone(),
                Tracked {
                    run: Run {
                        id: launch.run_id,
                        name: launch.workflow_name,
                        status: ActivityStatus::Running,
                        completed: 0,
                        total: 0,
                        elapsed_ms: 0,
                        tokens: 0,
                    }
                    .bounded(),
                    task: launch.task_id,
                    started,
                    journal: Cursor::default(),
                    workers: BTreeMap::new(),
                    final_seen: false,
                },
            );
            // Retain the newest runs; old completed history is not a transcript database.
            if self.runs.len() > MAX_RUNS {
                let oldest = self
                    .runs
                    .iter()
                    .min_by_key(|(_, r)| r.started)
                    .map(|(id, _)| id.clone())
                    .unwrap();
                self.runs.remove(&oldest);
            }
        }
        // Avoid replaying old launches while bootstrapping a large parent file.
        if std::fs::metadata(&self.parent).is_ok_and(|m| m.len() > self.cursor.offset) {
            return Vec::new();
        }
        for tracked in self.runs.values_mut() {
            if tracked.final_seen {
                continue;
            }
            let id = &tracked.run.id;
            let snapshot = self.root.join("workflows").join(format!("{id}.json"));
            if let Some(path) = safe_file(&self.root, &snapshot) {
                if let Ok(file) = File::open(path) {
                    if let Ok(saved) = serde_json::from_reader::<_, FinalRun>(file.take(CHUNK)) {
                        // The old killed snapshot survives a resume with the same run id.
                        if saved.run_id == *id && saved.task_id == tracked.task {
                            let status = match saved.status.as_str() {
                                "completed" => Some(ActivityStatus::Completed),
                                "failed" => Some(ActivityStatus::Failed),
                                "killed" | "stopped" | "cancelled" => Some(ActivityStatus::Stopped),
                                _ => None,
                            };
                            if let Some(status) = status {
                                let mut states = BTreeMap::new();
                                for p in saved.workflow_progress {
                                    if p.kind == "workflow_agent" {
                                        if let (Some(i), Some(s)) = (p.index, p.state) {
                                            states.insert(i, s);
                                        }
                                    }
                                }
                                tracked.run.status = status;
                                tracked.run.total = saved.agent_count;
                                tracked.run.completed =
                                    states.values().filter(|s| s.as_str() == "done").count() as u32;
                                tracked.run.tokens = saved.total_tokens;
                                tracked.run.elapsed_ms = saved.duration_ms;
                                tracked.final_seen = true;
                                tracked.workers.clear();
                                continue;
                            }
                        }
                    }
                }
            }
            let dir = self.root.join("subagents/workflows").join(id);
            if let Some(path) = safe_file(&self.root, &dir.join("journal.jsonl")) {
                for row in tracked.journal.read::<JournalRow>(&path) {
                    if row.kind == "launched" {
                        tracked.workers.clear();
                    }
                    let (Some(key), Some(agent)) = (row.key, row.agent_id) else {
                        continue;
                    };
                    if !identifier(&agent) || key.len() > 256 {
                        continue;
                    }
                    match row.kind.as_str() {
                        "started" if tracked.workers.len() < MAX_WORKERS => {
                            tracked.workers.insert(
                                key,
                                Worker {
                                    id: agent,
                                    ..Worker::default()
                                },
                            );
                        }
                        "result" => {
                            if let Some(worker) =
                                tracked.workers.get_mut(&key).filter(|w| w.id == agent)
                            {
                                worker.done = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            for worker in tracked.workers.values_mut() {
                if let Some(path) =
                    safe_file(&self.root, &dir.join(format!("agent-{}.jsonl", worker.id)))
                {
                    for row in worker.cursor.read::<WorkerRow>(&path) {
                        worker.update(row);
                    }
                }
            }
            // Resumes append to the same journal and keep cached results. Only
            // workers started in this attempt belong in its live counters. The
            // launch result is committed just after spawning, so allow 5s skew.
            let workers: Vec<_> = tracked
                .workers
                .values()
                .filter(|w| {
                    w.first_ms
                        .is_some_and(|t| t >= tracked.started.saturating_sub(5000))
                })
                .collect();
            tracked.run.total = workers.len() as u32;
            tracked.run.completed = workers.iter().filter(|w| w.done).count() as u32;
            tracked.run.tokens = workers
                .iter()
                .fold(0u64, |n, w| n.saturating_add(w.tokens()));
            tracked.run.elapsed_ms = now.saturating_sub(tracked.started) / 2000 * 2000;
        }
        let mut runs: Vec<_> = self.runs.values().collect();
        runs.sort_by_key(|r| {
            (
                r.run.status != ActivityStatus::Running,
                std::cmp::Reverse(r.started),
            )
        });
        runs.into_iter()
            .filter(|r| {
                r.run.status == ActivityStatus::Running
                    || now.saturating_sub(r.started.saturating_add(r.run.elapsed_ms)) < 300_000
            })
            .map(|r| r.run.clone().bounded())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "owt-workflows-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn append(&self, path: &str, value: serde_json::Value) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            writeln!(file, "{value}").unwrap();
        }
        fn launch(&self, task: &str, time: &str) {
            self.append("s.jsonl", json!({"sessionId":"s", "timestamp":time, "toolUseResult":{
                "status":"async_launched", "runId":"wf_one", "taskId":task, "workflowName":"Parallel checks",
                "transcriptDir":"/outside/session", "script":"never execute", "result":"do not retain"}}));
        }
        fn journal(&self, kind: &str, agent: &str) {
            self.append(
                "s/subagents/workflows/wf_one/journal.jsonl",
                json!({"type":kind,"key":"job","agentId":agent}),
            );
        }
        fn worker(&self, agent: &str, id: &str, time: &str, input: u64, output: u64) {
            self.append(&format!("s/subagents/workflows/wf_one/agent-{agent}.jsonl"), json!({
                "timestamp":time,"type":"assistant","message":{"id":id,"usage":{"input_tokens":input,
                "cache_read_input_tokens":100, "cache_creation_input_tokens":20,"output_tokens":output},
                "content":[{"type":"thinking","thinking":"private text must never be stored"}]}}));
        }
        fn final_run(&self, task: &str, status: &str) {
            self.append(
                "s/workflows/wf_one.json",
                json!({"runId":"wf_one","taskId":task,"status":status,
                "durationMs":9000,"agentCount":1,"totalTokens":900,
                "workflowProgress":[{"type":"workflow_agent","index":1,"state":"start"},
                    {"type":"workflow_agent","index":1,"state":"done"}]}),
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    const START: &str = "2026-10-13T13:00:00Z";
    const RESUME: &str = "2026-10-13T13:10:00Z";

    #[test]
    fn running_workers_update_without_parent_messages_and_deduplicate_usage() {
        let f = Fixture::new();
        f.launch("task1", START);
        f.journal("started", "a1");
        f.worker("a1", "m1", START, 10, 5);
        f.worker("a1", "m1", START, 10, 5);
        let mut tracker = Tracker::new(&f.0.join("s.jsonl"), "s");
        let now = timestamp_ms(START).unwrap() + 2000;
        let runs = tracker.poll(now);
        assert_eq!(
            (runs[0].completed, runs[0].total, runs[0].tokens),
            (0, 1, 135)
        );
        assert_eq!(runs[0].elapsed_ms, 2000);
        f.worker("a1", "m2", START, 30, 7);
        f.journal("result", "a1");
        let runs = tracker.poll(now + 2000);
        assert_eq!(
            (runs[0].completed, runs[0].total, runs[0].tokens),
            (1, 1, 162)
        );
        // A worker finishing does not finish the whole workflow.
        assert_eq!(runs[0].status, ActivityStatus::Running);
        f.final_run("task1", "completed");
        let runs = tracker.poll(now + 10_000);
        assert_eq!(runs[0].status, ActivityStatus::Completed);
        assert_eq!(
            (runs[0].completed, runs[0].tokens, runs[0].elapsed_ms),
            (1, 900, 9000)
        );
        assert!(tracker.poll(now + 400_000).is_empty());
    }

    #[test]
    fn restart_recovers_resumes_without_stale_final_or_cached_workers() {
        let f = Fixture::new();
        f.launch("old", START);
        f.journal("started", "oldagent");
        f.worker("oldagent", "oldmessage", START, 999, 10);
        f.journal("result", "oldagent");
        f.final_run("old", "killed");
        f.launch("new", RESUME);
        // A resume reuses the journal without a second `launched` record.
        f.journal("started", "newagent");
        f.worker("newagent", "newmessage", RESUME, 10, 5);
        f.journal("result", "oldagent"); // late old result must not complete the new worker
        let mut tracker = Tracker::new(&f.0.join("s.jsonl"), "s");
        let runs = tracker.poll(timestamp_ms(RESUME).unwrap() + 4000);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, ActivityStatus::Running);
        assert_eq!(
            (runs[0].completed, runs[0].total, runs[0].tokens),
            (0, 1, 135)
        );
        assert_eq!(runs[0].elapsed_ms, 4000);
    }

    #[test]
    fn malformed_partial_foreign_and_escaping_records_are_ignored() {
        let f = Fixture::new();
        f.append(
            "s.jsonl",
            json!({"sessionId":"other", "timestamp":START,"toolUseResult":{
            "status":"async_launched","runId":"wf_other","taskId":"t","workflowName":"Foreign"}}),
        );
        f.append(
            "s.jsonl",
            json!({"sessionId":"s", "timestamp":START,"toolUseResult":{
            "status":"async_launched","runId":"../escape","taskId":"t","workflowName":"Escape"}}),
        );
        let mut tracker = Tracker::new(&f.0.join("s.jsonl"), "s");
        assert!(tracker.poll(0).is_empty());
        let path = f.0.join("partial.jsonl");
        std::fs::write(&path, b"{\"message\":").unwrap();
        let mut cursor = Cursor::default();
        assert!(cursor.read::<WorkerRow>(&path).is_empty());
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"null}\n").unwrap();
        assert_eq!(cursor.read::<WorkerRow>(&path).len(), 1);
        #[cfg(unix)]
        {
            let outside = f.0.join("outside.jsonl");
            std::fs::write(&outside, "{}\n").unwrap();
            std::fs::create_dir_all(f.0.join("s")).unwrap();
            std::os::unix::fs::symlink(&outside, f.0.join("s/escape.jsonl")).unwrap();
            assert!(safe_file(&f.0.join("s"), &f.0.join("s/escape.jsonl")).is_none());
        }
    }
}
