//! `[voice.tts] engine = "mlx"`: a neural text-to-speech model running on
//! this Mac's GPU through MLX, in a Python sidecar the daemon owns
//! (`host/tts/sidecar.py`, embedded in the binary and written out beside
//! the model cache), behind §7.6's `TtsEngine` seam like the Apple one.
//!
//! The sidecar loads the model once and keeps it; the daemon starts it at
//! boot so the first announcement does not pay the load. Each utterance is
//! one `say` request on its stdin and comes back as PCM frames on its
//! stdout, streamed as the model produces them, so playback starts after
//! the first chunk rather than after the whole sentence. Barge-in is a
//! `cancel` line; the sidecar stops at its next chunk.
//!
//! While the sidecar is not ready — still loading, not installed, or it
//! died — an utterance goes to the fallback engine (Apple's) instead of
//! being dropped, and the log says why. `deckd tts install` builds the
//! Python environment and downloads the model; `deckd tts status` reports.
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};

use super::traits::{
    AudioError, AudioFormat, EngineInfo, TtsChunk, TtsEngine, TtsJob, TtsRequest, TtsStop,
    VoiceInfo,
};

/// The sidecar script, as shipped with this build of the daemon.
pub const SCRIPT: &str = include_str!("../../../../tts/sidecar.py");
/// The engine's name in `voice.engines` and `[voice.tts] engine`.
pub const NAME: &str = "mlx";
/// The model `deckd tts install` fetches when the config names none.
pub const DEFAULT_MODEL: &str = "mlx-community/Qwen3-TTS-12Hz-1.7B-CustomVoice-8bit";
/// The speakers agents are assigned from when `[voice.tts] voices` names
/// none: four of [`DEFAULT_MODEL`]'s seven (`Uncle_Fu`, `Dylan` and `Eric`
/// are the others).
pub const DEFAULT_VOICES: [&str; 4] = ["Ryan", "Aiden", "Vivian", "Serena"];
/// How long `synthesize` waits for a sidecar that is still loading before
/// handing the utterance to the fallback.
const READY_WAIT: Duration = Duration::from_millis(1500);
/// A sidecar that died is restarted at the next utterance, not before this.
const RESPAWN_AFTER: Duration = Duration::from_secs(10);
/// What the Qwen3-TTS checkpoints speak, as `voice.engines` reports it.
pub const LOCALES: [&str; 10] = [
    "en-US", "zh-CN", "ja-JP", "ko-KR", "de-DE", "fr-FR", "es-ES", "it-IT", "pt-BR", "ru-RU",
];

/// What the engine is built from (`[voice.tts]` + the paths).
#[derive(Clone, Debug, PartialEq)]
pub struct SidecarConfig {
    /// The Python of the sidecar's virtual environment.
    pub python: PathBuf,
    /// Where the embedded script is written and run from.
    pub script: PathBuf,
    pub model: String,
    /// The voices agents are assigned from; empty means the model's own
    /// list, as the sidecar reports it.
    pub voices: Vec<String>,
    /// `lang_code`: `"auto"` lets the model detect it.
    pub lang: String,
    /// A delivery instruction sent with every utterance, for the models
    /// that take one (Qwen3-TTS CustomVoice: emotion and style).
    pub instruct: Option<String>,
    /// Seconds of audio per streamed chunk.
    pub interval_s: f32,
}

/// The `R` frame: what the sidecar loaded.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Ready {
    pub model: String,
    pub rate: u32,
    #[serde(default = "one")]
    pub channels: u16,
    #[serde(default)]
    pub voices: Vec<String>,
    #[serde(default)]
    pub load_ms: u64,
}

fn one() -> u16 {
    1
}

/// Where the sidecar stands, for `deckd tts status` and the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Health {
    /// Not started yet, or restarting.
    Starting,
    Ready,
    /// It could not start or it died: the fallback speaks, this is why.
    Down(String),
}

struct State {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    ready: Option<Ready>,
    health: Health,
    /// Jobs in flight, by the id the sidecar echoes.
    jobs: HashMap<u64, Sender<TtsChunk>>,
    next_id: u64,
    /// When the sidecar last died, for the respawn delay.
    died_at: Option<Instant>,
    spawning: bool,
}

struct Inner {
    cfg: SidecarConfig,
    state: Mutex<State>,
    changed: Condvar,
}

/// The engine.
pub struct SidecarTts {
    inner: Arc<Inner>,
    fallback: Option<Arc<dyn TtsEngine>>,
}

impl std::fmt::Debug for SidecarTts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidecarTts")
            .field("model", &self.inner.cfg.model)
            .finish_non_exhaustive()
    }
}

impl SidecarTts {
    /// Build the engine and start the sidecar in the background.
    /// `fallback` speaks whatever the sidecar cannot.
    #[must_use]
    pub fn new(cfg: SidecarConfig, fallback: Option<Arc<dyn TtsEngine>>) -> Self {
        let inner = Arc::new(Inner {
            cfg,
            state: Mutex::new(State {
                child: None,
                stdin: None,
                ready: None,
                health: Health::Starting,
                jobs: HashMap::new(),
                next_id: 1,
                died_at: None,
                spawning: false,
            }),
            changed: Condvar::new(),
        });
        Inner::spawn_in_background(&inner);
        SidecarTts { inner, fallback }
    }

    /// The sidecar's state right now.
    #[must_use]
    pub fn health(&self) -> Health {
        self.inner.state.lock().unwrap().health.clone()
    }

    /// Block until the sidecar is ready or down, at most `timeout`.
    /// `true` when it is ready.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        self.inner.wait_ready(timeout)
    }

    /// The voice the sidecar is asked for: the agent's own when the model
    /// has it, else one chosen for that name and stable across restarts,
    /// so two agents keep two voices even when the pool names are Apple's.
    #[must_use]
    pub fn map_voice(voices: &[String], requested: Option<&str>) -> Option<String> {
        if voices.is_empty() {
            return requested.map(str::to_string);
        }
        let Some(name) = requested.filter(|n| !n.trim().is_empty()) else {
            return voices.first().cloned();
        };
        if let Some(exact) = voices.iter().find(|v| v.eq_ignore_ascii_case(name)) {
            return Some(exact.clone());
        }
        // FNV-1a over the name, then a finalizer (FNV's low bits alone are
        // poor for a modulo): the same agent name lands on the same voice
        // every time, and names spread across the list.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in name.bytes() {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        hash ^= hash >> 33;
        let index = usize::try_from(hash % voices.len() as u64).unwrap_or(0);
        voices.get(index).cloned()
    }

    fn voice_list(&self, ready: Option<&Ready>) -> Vec<String> {
        if !self.inner.cfg.voices.is_empty() {
            return self.inner.cfg.voices.clone();
        }
        ready.map(|r| r.voices.clone()).unwrap_or_default()
    }
}

impl Inner {
    fn spawn_in_background(inner: &Arc<Inner>) {
        {
            let mut st = inner.state.lock().unwrap();
            if st.spawning || st.child.is_some() {
                return;
            }
            st.spawning = true;
            st.health = Health::Starting;
        }
        let me = inner.clone();
        let _ = std::thread::Builder::new()
            .name("deckd-tts-sidecar".into())
            .spawn(move || me.spawn());
    }

    /// Write the script, start the process, read its frames until it ends.
    fn spawn(self: Arc<Self>) {
        let result = self.start();
        let mut st = self.state.lock().unwrap();
        st.spawning = false;
        if let Err(why) = result {
            tracing::warn!(why, "mlx tts: not started; the fallback engine speaks");
            st.health = Health::Down(why);
            st.died_at = Some(Instant::now());
            self.changed.notify_all();
        }
    }

    fn start(self: &Arc<Self>) -> Result<(), String> {
        let cfg = &self.cfg;
        if !cfg.python.exists() {
            return Err(format!(
                "no Python at {} (run `deckd tts install`)",
                cfg.python.display()
            ));
        }
        if !cfg.script.is_file() {
            return Err(format!("no sidecar script at {}", cfg.script.display()));
        }
        let mut child = Command::new(&cfg.python)
            .arg(&cfg.script)
            .arg("--model")
            .arg(&cfg.model)
            .env("PYTHONUNBUFFERED", "1")
            .env("HF_HUB_DISABLE_PROGRESS_BARS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", cfg.python.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let pid = child.id();
        tracing::info!(pid, model = %cfg.model, "mlx tts: sidecar started, loading the model");
        {
            let mut st = self.state.lock().unwrap();
            st.child = Some(child);
            st.stdin = Some(stdin);
        }
        let _ = std::thread::Builder::new()
            .name("deckd-tts-sidecar-err".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let lower = line.to_ascii_lowercase();
                    if lower.contains("error") || lower.contains("traceback") || lower.contains("failed") {
                        tracing::warn!(pid, "mlx tts: {line}");
                    } else {
                        tracing::debug!(pid, "mlx tts: {line}");
                    }
                }
            });
        let me = self.clone();
        let _ = std::thread::Builder::new()
            .name("deckd-tts-sidecar-out".into())
            .spawn(move || {
                let why = me.read_frames(stdout);
                me.ended(why);
            });
        Ok(())
    }

    /// The sidecar's stdout, frame by frame, until it closes.
    fn read_frames(&self, stdout: impl Read) -> String {
        let mut rd = BufReader::new(stdout);
        let mut current: Option<u64> = None;
        loop {
            let mut head = [0u8; 5];
            if let Err(e) = rd.read_exact(&mut head) {
                return if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    "the sidecar exited".into()
                } else {
                    format!("reading the sidecar: {e}")
                };
            }
            let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
            if len > 64 << 20 {
                return format!("a {len}-byte frame: the stream is out of sync");
            }
            let mut payload = vec![0u8; len];
            if let Err(e) = rd.read_exact(&mut payload) {
                return format!("reading a frame: {e}");
            }
            match head[0] {
                b'R' => match serde_json::from_slice::<Ready>(&payload) {
                    Ok(ready) => {
                        tracing::info!(
                            model = %ready.model,
                            rate = ready.rate,
                            voices = ready.voices.len(),
                            load_ms = ready.load_ms,
                            "mlx tts: ready"
                        );
                        let mut st = self.state.lock().unwrap();
                        st.ready = Some(ready);
                        st.health = Health::Ready;
                        self.changed.notify_all();
                    }
                    Err(e) => return format!("an unreadable R frame: {e}"),
                },
                b'H' => {
                    let v: serde_json::Value = serde_json::from_slice(&payload).unwrap_or_default();
                    current = v["id"].as_u64();
                    tracing::debug!(id = current, first_ms = v["first_ms"].as_u64(), "mlx tts: audio starts");
                }
                b'P' => {
                    let Some(id) = current else { continue };
                    let samples: Vec<f32> = payload
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|p| f32::from(i16::from_le_bytes(*p)) / 32768.0)
                        .collect();
                    let st = self.state.lock().unwrap();
                    if let Some(tx) = st.jobs.get(&id) {
                        let _ = tx.send(TtsChunk::Pcm(samples));
                    }
                }
                b'E' | b'X' => {
                    let v: serde_json::Value = serde_json::from_slice(&payload).unwrap_or_default();
                    let Some(id) = v["id"].as_u64() else { continue };
                    if current == Some(id) {
                        current = None;
                    }
                    let chunk = if head[0] == b'E' {
                        tracing::debug!(
                            id,
                            ms = v["ms"].as_u64(),
                            samples = v["samples"].as_u64(),
                            cancelled = v["cancelled"].as_bool(),
                            "mlx tts: utterance ended"
                        );
                        TtsChunk::End
                    } else {
                        let error = v["error"].as_str().unwrap_or("unknown").to_string();
                        tracing::warn!(id, error, "mlx tts: utterance failed");
                        TtsChunk::Error(format!("mlx tts: {error}"))
                    };
                    let mut st = self.state.lock().unwrap();
                    if let Some(tx) = st.jobs.remove(&id) {
                        let _ = tx.send(chunk);
                    }
                }
                other => tracing::debug!(tag = other, len, "mlx tts: an unknown frame"),
            }
        }
    }

    /// The process is gone: every job in flight fails, the next utterance
    /// restarts it after a pause.
    fn ended(&self, why: String) {
        let mut st = self.state.lock().unwrap();
        let code = st
            .child
            .as_mut()
            .and_then(|c| c.try_wait().ok().flatten())
            .and_then(|s| s.code());
        tracing::warn!(why, code, "mlx tts: sidecar ended; the fallback engine speaks until it is back");
        st.child = None;
        st.stdin = None;
        st.ready = None;
        st.health = Health::Down(why.clone());
        st.died_at = Some(Instant::now());
        for (_, tx) in st.jobs.drain() {
            let _ = tx.send(TtsChunk::Error(format!("mlx tts: {why}")));
        }
        self.changed.notify_all();
    }

    fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = self.state.lock().unwrap();
        loop {
            match st.health {
                Health::Ready => return true,
                Health::Down(_) if !st.spawning => return false,
                _ => {}
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, _) = self.changed.wait_timeout(st, deadline - now).unwrap();
            st = guard;
        }
    }

    fn write_line(&self, line: &str) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let stdin = st.stdin.as_mut().ok_or("the sidecar is not running")?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("writing to the sidecar: {e}"))
    }
}

/// Write the embedded script where the engine runs it from, when it is
/// not already that exact text.
///
/// # Errors
/// What the file system said.
pub fn write_script(path: &Path) -> Result<(), String> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(SCRIPT) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, SCRIPT).map_err(|e| format!("{}: {e}", path.display()))
}

struct SidecarJob {
    format: AudioFormat,
    chunks: Receiver<TtsChunk>,
    stopper: Arc<SidecarStopper>,
}

impl TtsJob for SidecarJob {
    fn format(&self) -> AudioFormat {
        self.format
    }

    fn chunks(&self) -> Receiver<TtsChunk> {
        self.chunks.clone()
    }

    fn stopper(&self) -> Arc<dyn TtsStop> {
        self.stopper.clone()
    }
}

struct SidecarStopper {
    inner: Arc<Inner>,
    id: u64,
}

impl TtsStop for SidecarStopper {
    fn stop(&self) {
        let still_running = self.inner.state.lock().unwrap().jobs.contains_key(&self.id);
        if still_running {
            let _ = self
                .inner
                .write_line(&format!("{{\"op\":\"cancel\",\"id\":{}}}", self.id));
        }
    }
}

impl TtsEngine for SidecarTts {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            name: NAME.into(),
            on_device: true,
            locales: LOCALES.iter().map(|l| (*l).to_string()).collect(),
        }
    }

    fn voices(&self) -> Vec<VoiceInfo> {
        let ready = self.inner.state.lock().unwrap().ready.clone();
        self.voice_list(ready.as_ref())
            .into_iter()
            .map(|id| VoiceInfo {
                name: id.clone(),
                id,
                locale: String::new(),
                enhanced: true,
            })
            .collect()
    }

    fn synthesize(&self, req: &TtsRequest) -> Result<Box<dyn TtsJob>, AudioError> {
        if !self.inner.wait_ready(READY_WAIT) {
            let (health, died_at, spawning) = {
                let st = self.inner.state.lock().unwrap();
                (st.health.clone(), st.died_at, st.spawning)
            };
            if let Health::Down(_) = &health {
                if !spawning && died_at.is_none_or(|at| at.elapsed() >= RESPAWN_AFTER) {
                    Inner::spawn_in_background(&self.inner);
                }
            }
            let why = match &health {
                Health::Starting => "still loading".to_string(),
                Health::Down(why) => why.clone(),
                Health::Ready => "not ready".to_string(),
            };
            return match &self.fallback {
                Some(apple) => {
                    tracing::info!(why, "mlx tts: the fallback engine speaks this one");
                    apple.synthesize(req)
                }
                None => Err(AudioError::Engine(format!("mlx tts: {why}"))),
            };
        }
        let (id, format, voice) = {
            let mut st = self.inner.state.lock().unwrap();
            let ready = st.ready.clone().ok_or_else(|| AudioError::Engine("mlx tts: not ready".into()))?;
            let id = st.next_id;
            st.next_id += 1;
            let voice = Self::map_voice(&self.voice_list(Some(&ready)), req.voice.as_deref());
            (id, AudioFormat::new(ready.rate, ready.channels), voice)
        };
        let mut line = serde_json::json!({
            "op": "say",
            "id": id,
            "text": req.text,
            "lang": self.inner.cfg.lang,
            "interval": self.inner.cfg.interval_s,
        });
        if let Some(voice) = voice {
            line["voice"] = serde_json::Value::String(voice);
        }
        if let Some(instruct) = &self.inner.cfg.instruct {
            line["instruct"] = serde_json::Value::String(instruct.clone());
        }
        // AVSpeech's 0.5 is the normal rate; the sidecar's `speed` is 1.0.
        if (req.rate - 0.5).abs() > 0.05 {
            line["speed"] = serde_json::json!((req.rate / 0.5).clamp(0.5, 2.0));
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        self.inner.state.lock().unwrap().jobs.insert(id, tx);
        if let Err(why) = self.inner.write_line(&line.to_string()) {
            self.inner.state.lock().unwrap().jobs.remove(&id);
            return Err(AudioError::Engine(format!("mlx tts: {why}")));
        }
        Ok(Box::new(SidecarJob {
            format,
            chunks: rx,
            stopper: Arc::new(SidecarStopper {
                inner: self.inner.clone(),
                id,
            }),
        }))
    }
}

impl Drop for SidecarTts {
    fn drop(&mut self) {
        let _ = self.inner.write_line("{\"op\":\"quit\"}");
        let mut st = self.inner.state.lock().unwrap();
        st.stdin = None;
        if let Some(mut child) = st.child.take() {
            // The quit line ends it; a sidecar stuck in the model gets the signal.
            std::thread::sleep(Duration::from_millis(200));
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in sidecar that speaks the protocol with a synthetic tone:
    /// what the engine is tested against on every platform with a Python.
    const FAKE: &str = r#"
import json, struct, sys, threading, time
def frame(tag, payload):
    sys.stdout.buffer.write(tag + struct.pack("<I", len(payload)) + payload); sys.stdout.buffer.flush()
def fj(tag, obj): frame(tag, json.dumps(obj).encode())
args = sys.argv[1:]
model = args[args.index("--model") + 1] if "--model" in args else "?"
if model == "crash-on-load":
    sys.stderr.write("sidecar: cannot load\n"); sys.exit(3)
time.sleep(0.05)
fj(b"R", {"model": model, "rate": 24000, "channels": 1, "voices": ["Ryan", "Aiden", "Vivian"], "load_ms": 50})
cancelled = set()
lock = threading.Lock()
queue = []
def reader():
    for line in sys.stdin.buffer:
        req = json.loads(line)
        with lock:
            if req["op"] == "say": queue.append(req)
            elif req["op"] == "cancel": cancelled.add(req["id"])
            elif req["op"] == "quit": queue.append(None)
threading.Thread(target=reader, daemon=True).start()
while True:
    with lock:
        req = queue.pop(0) if queue else False
    if req is False:
        time.sleep(0.005); continue
    if req is None: break
    fj(b"H", {"id": req["id"], "first_ms": 1})
    if req["text"] == "fail":
        fj(b"X", {"id": req["id"], "error": "boom"}); continue
    n = 10 if req["text"] == "long" else 2
    sent = 0
    for i in range(n):
        with lock:
            if req["id"] in cancelled: break
        pcm = struct.pack("<3h", 1000 * (i + 1), -1000 * (i + 1), 0)
        frame(b"P", pcm); sent += 3
        time.sleep(0.03)
    fj(b"E", {"id": req["id"], "ms": 1, "samples": sent, "cancelled": req["id"] in cancelled, "voice": req.get("voice")})
"#;

    fn python() -> Option<PathBuf> {
        ["/usr/bin/python3", "/usr/local/bin/python3", "/opt/homebrew/bin/python3"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
    }

    fn fake_cfg(model: &str) -> Option<SidecarConfig> {
        let dir = std::env::temp_dir().join(format!("deckd-sidecar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let script = dir.join("fake.py");
        std::fs::write(&script, FAKE).ok()?;
        Some(SidecarConfig {
            python: python()?,
            script,
            model: model.into(),
            voices: Vec::new(),
            lang: "auto".into(),
            instruct: None,
            interval_s: 0.32,
        })
    }

    fn drain(job: &dyn TtsJob) -> (Vec<f32>, Option<TtsChunk>) {
        let rx = job.chunks();
        let mut pcm = Vec::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(5)) {
            match chunk {
                TtsChunk::Pcm(s) => pcm.extend(s),
                other => return (pcm, Some(other)),
            }
        }
        (pcm, None)
    }

    #[test]
    fn voices_map_exactly_or_stably_by_name() {
        let voices: Vec<String> = ["Ryan", "Aiden", "Vivian", "Serena"].map(String::from).to_vec();
        assert_eq!(SidecarTts::map_voice(&voices, Some("aiden")).as_deref(), Some("Aiden"));
        assert_eq!(SidecarTts::map_voice(&voices, None).as_deref(), Some("Ryan"));
        assert_eq!(SidecarTts::map_voice(&voices, Some("  ")).as_deref(), Some("Ryan"));
        let a = SidecarTts::map_voice(&voices, Some("Samantha")).unwrap();
        let b = SidecarTts::map_voice(&voices, Some("Samantha")).unwrap();
        assert_eq!(a, b, "stable");
        assert!(voices.contains(&a));
        let names = ["Samantha", "Daniel", "Karen", "Rishi"];
        let mapped: std::collections::BTreeSet<String> = names
            .iter()
            .map(|n| SidecarTts::map_voice(&voices, Some(n)).unwrap())
            .collect();
        assert!(mapped.len() >= 2, "four pool names do not all collapse onto one voice: {mapped:?}");
        assert_eq!(SidecarTts::map_voice(&[], Some("Kore")).as_deref(), Some("Kore"));
        assert_eq!(SidecarTts::map_voice(&[], None), None);
    }

    #[test]
    fn the_script_is_written_once_and_kept_identical() {
        let dir = std::env::temp_dir().join(format!("deckd-sidecar-script-{}", std::process::id()));
        let path = dir.join("tts").join("sidecar.py");
        write_script(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SCRIPT);
        std::fs::write(&path, "stale").unwrap();
        write_script(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SCRIPT);
        assert!(SCRIPT.contains("\"op\":"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn utterances_stream_end_fail_and_cancel_through_a_fake_sidecar() {
        let Some(cfg) = fake_cfg("fake-model") else {
            eprintln!("no python3: skipped");
            return;
        };
        let engine = SidecarTts::new(cfg, None);
        assert!(engine.wait_ready(Duration::from_secs(10)), "{:?}", engine.health());
        assert_eq!(engine.health(), Health::Ready);
        assert_eq!(engine.info().name, "mlx");
        assert_eq!(
            engine.voices().iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            ["Ryan", "Aiden", "Vivian"]
        );

        let job = engine
            .synthesize(&TtsRequest {
                text: "hello".into(),
                voice: Some("vivian".into()),
                rate: 0.5,
            })
            .unwrap();
        assert_eq!(job.format(), AudioFormat::new(24_000, 1));
        let (pcm, last) = drain(job.as_ref());
        assert_eq!(last, Some(TtsChunk::End));
        assert_eq!(pcm.len(), 6);
        assert!((pcm[0] - 1000.0 / 32768.0).abs() < 1e-6);
        assert!((pcm[4] + 2000.0 / 32768.0).abs() < 1e-6);

        let job = engine
            .synthesize(&TtsRequest {
                text: "fail".into(),
                voice: None,
                rate: 0.5,
            })
            .unwrap();
        let (pcm, last) = drain(job.as_ref());
        assert!(pcm.is_empty());
        assert!(matches!(last, Some(TtsChunk::Error(e)) if e.contains("boom")));

        // A cancel mid-stream: fewer than the ten chunks, then an End.
        let job = engine
            .synthesize(&TtsRequest {
                text: "long".into(),
                voice: None,
                rate: 0.5,
            })
            .unwrap();
        let rx = job.chunks();
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(first, TtsChunk::Pcm(_)));
        job.stopper().stop();
        let mut chunks = 1;
        let mut ended = false;
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(5)) {
            match chunk {
                TtsChunk::Pcm(_) => chunks += 1,
                TtsChunk::End => {
                    ended = true;
                    break;
                }
                TtsChunk::Error(e) => panic!("{e}"),
            }
        }
        assert!(ended);
        assert!(chunks < 10, "cancel cut the stream: {chunks} chunks");
        // Stopping an ended job is harmless.
        job.stopper().stop();
        drop(engine);
    }

    #[test]
    fn a_sidecar_that_cannot_load_leaves_the_fallback_speaking() {
        let Some(cfg) = fake_cfg("crash-on-load") else {
            return;
        };
        let (fake, _handle) = super::super::fakes::FakeTts::new();
        let engine = SidecarTts::new(cfg, Some(Arc::new(fake)));
        assert!(!engine.wait_ready(Duration::from_secs(10)));
        assert!(matches!(engine.health(), Health::Down(_)), "{:?}", engine.health());
        // The fallback takes the utterance rather than it being dropped.
        let job = engine.synthesize(&TtsRequest {
            text: "hello".into(),
            voice: None,
            rate: 0.5,
        });
        assert!(job.is_ok(), "{:?}", job.err());
        // No Python at all: down with the install hint.
        let mut cfg = fake_cfg("fake-model").unwrap();
        cfg.python = PathBuf::from("/nonexistent/python3");
        let engine = SidecarTts::new(cfg, None);
        assert!(!engine.wait_ready(Duration::from_secs(5)));
        match engine.health() {
            Health::Down(why) => assert!(why.contains("deckd tts install"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert!(engine
            .synthesize(&TtsRequest {
                text: "x".into(),
                voice: None,
                rate: 0.5
            })
            .is_err());
    }
}
