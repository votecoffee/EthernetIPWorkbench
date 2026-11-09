//! Read-only telemetry fan-out. The receiver only copies into a bounded queue.

mod http;
mod mqtt;
#[cfg(windows)]
mod shared_memory;

use crate::model::{MAX_ASSEMBLY_BYTES, Settings, hex};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    net::{Ipv4Addr, TcpListener},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
    },
    thread::{self, JoinHandle, Thread},
    time::{Duration, Instant},
};

const CAPACITY: usize = 1024;
const IDLE: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub http_port: Option<u16>,
    pub mqtt: Option<MqttOptions>,
    pub shared_memory: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttOptions {
    pub broker: String,
    pub port: u16,
    pub topic: String,
    pub period_ms: u64,
}

impl Options {
    pub fn is_disabled(&self) -> bool {
        self.http_port.is_none() && self.mqtt.is_none() && self.shared_memory.is_none()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.http_port == Some(0) {
            return Err("Integration HTTP port must be nonzero".into());
        }
        if let Some(name) = &self.shared_memory {
            if !cfg!(windows) {
                return Err("Named shared memory is supported on Windows".into());
            }
            if !(1..=64).contains(&name.len())
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
            {
                return Err(
                    "Shared memory name must be 1–64 letters, digits, hyphens or underscores"
                        .into(),
                );
            }
        }
        if let Some(mqtt) = &self.mqtt {
            if mqtt.broker.is_empty()
                || mqtt
                    .broker
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control())
                || mqtt.port == 0
            {
                return Err("MQTT requires a broker host and a nonzero port".into());
            }
            if !(1..=60_000).contains(&mqtt.period_ms) {
                return Err("MQTT publish period must be 1–60,000 ms".into());
            }
            if !(1..=512).contains(&mqtt.topic.len())
                || mqtt
                    .topic
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '#' | '+'))
            {
                return Err("MQTT requires a nonempty publish topic without wildcards".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Frame {
    index: u64,
    generation: u64,
    elapsed_us: u64,
    packet_sequence: u32,
    cip_sequence: u16,
    size: u16,
    data: [u8; MAX_ASSEMBLY_BYTES],
}

impl Frame {
    fn bytes(&self) -> &[u8] {
        &self.data[..usize::from(self.size)]
    }
}

struct Feed {
    origin: Instant,
    started_utc_us: i64,
    settings: Settings,
    queue: ArrayQueue<Frame>,
    history: Mutex<VecDeque<Frame>>,
    dispatcher: OnceLock<Thread>,
    stop: AtomicBool,
    active_generation: AtomicU64,
    received: AtomicU64,
    queue_drops: AtomicU64,
    mqtt_connected: AtomicBool,
    mqtt_errors: AtomicU64,
    memory_busy: AtomicU64,
}

impl Feed {
    fn frame_json(&self, frame: Frame) -> Value {
        let bytes = frame.bytes();
        let fields = &self.settings.capture_fields;
        let value = |selection: crate::model::Selection| {
            selection.decode(bytes).map(|value| value.to_string())
        };
        let values: serde_json::Map<String, Value> = if fields.is_empty() {
            [("value".into(), json!(value(self.settings.selection)))]
                .into_iter()
                .collect()
        } else {
            fields
                .iter()
                .map(|field| (field.name.clone(), json!(value(field.selection))))
                .collect()
        };
        json!({
            "schema_version": 1,
            "stream_start_utc_us": self.started_utc_us,
            "sample_index": frame.index,
            "generation": frame.generation,
            "elapsed_us": frame.elapsed_us,
            "packet_sequence": frame.packet_sequence,
            "cip_sequence": frame.cip_sequence,
            "input_assembly": self.settings.input_instance,
            "payload_hex": hex(bytes),
            "values": values,
        })
    }

    fn latest(&self) -> Option<Frame> {
        self.history.lock().unwrap().back().copied()
    }

    fn wake_dispatcher(&self) {
        if let Some(worker) = self.dispatcher.get() {
            worker.unpark();
        }
    }
}

#[derive(Clone)]
pub struct Publisher(Arc<Feed>);

impl Publisher {
    pub(crate) fn set_active_generation(&self, generation: u64) {
        self.0.active_generation.store(generation, Relaxed);
        self.0.wake_dispatcher();
    }

    pub(crate) fn record(
        &self,
        stamp: Instant,
        generation: u64,
        sequence: u32,
        cip: u16,
        bytes: &[u8],
    ) {
        let feed = &self.0;
        let mut data = [0; MAX_ASSEMBLY_BYTES];
        data[..bytes.len()].copy_from_slice(bytes);
        let frame = Frame {
            index: feed.received.fetch_add(1, Relaxed) + 1,
            generation,
            elapsed_us: stamp.saturating_duration_since(feed.origin).as_micros() as u64,
            packet_sequence: sequence,
            cip_sequence: cip,
            size: bytes.len() as u16,
            data,
        };
        if feed.queue.force_push(frame).is_some() {
            feed.queue_drops.fetch_add(1, Relaxed);
        }
        feed.wake_dispatcher();
    }
}

#[derive(Clone, Copy, Serialize)]
pub struct Counters {
    pub received: u64,
    pub queue_drops: u64,
    pub mqtt_connected: bool,
    pub mqtt_errors: u64,
    pub shared_memory_busy: u64,
}

pub struct Integration {
    feed: Arc<Feed>,
    workers: Vec<JoinHandle<()>>,
}

impl Integration {
    pub fn start(options: &Options, settings: &Settings) -> Result<Option<Self>, String> {
        options.validate()?;
        if options.is_disabled() {
            return Ok(None);
        }
        let listener = options
            .http_port
            .map(|port| {
                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
                listener.set_nonblocking(true)?;
                Ok::<_, std::io::Error>(listener)
            })
            .transpose()
            .map_err(|error| format!("Integration HTTP bind failed: {error}"))?;
        #[cfg(windows)]
        let memory = options
            .shared_memory
            .as_deref()
            .map(shared_memory::Mapping::new)
            .transpose()
            .map_err(|error| format!("Integration shared memory setup failed: {error}"))?;
        let feed = Arc::new(Feed {
            origin: Instant::now(),
            started_utc_us: chrono::Utc::now().timestamp_micros(),
            settings: settings.clone(),
            queue: ArrayQueue::new(CAPACITY),
            history: Mutex::new(VecDeque::with_capacity(CAPACITY)),
            dispatcher: OnceLock::new(),
            stop: AtomicBool::new(false),
            active_generation: AtomicU64::new(0),
            received: AtomicU64::new(0),
            queue_drops: AtomicU64::new(0),
            mqtt_connected: AtomicBool::new(false),
            mqtt_errors: AtomicU64::new(0),
            memory_busy: AtomicU64::new(0),
        });
        let mut service = Self {
            feed: feed.clone(),
            workers: Vec::new(),
        };
        service.spawn("integration-dispatch", move |feed| {
            dispatch(
                &feed,
                #[cfg(windows)]
                memory,
            )
        })?;
        feed.dispatcher
            .set(service.workers[0].thread().clone())
            .expect("new dispatcher");
        if let Some(listener) = listener {
            service.spawn("integration-http", move |feed| http::serve(&feed, listener))?;
        }
        if let Some(mqtt) = options.mqtt.clone() {
            service.spawn("integration-mqtt", move |feed| mqtt::publish(&feed, mqtt))?;
        }
        Ok(Some(service))
    }

    fn spawn(
        &mut self,
        name: &str,
        run: impl FnOnce(Arc<Feed>) + Send + 'static,
    ) -> Result<(), String> {
        let feed = self.feed.clone();
        self.workers.push(
            thread::Builder::new()
                .name(name.into())
                .spawn(move || run(feed))
                .map_err(|error| format!("Cannot start {name}: {error}"))?,
        );
        Ok(())
    }

    pub fn publisher(&self) -> Publisher {
        Publisher(self.feed.clone())
    }

    pub fn counters(&self) -> Counters {
        counters(&self.feed)
    }
}

fn counters(feed: &Feed) -> Counters {
    Counters {
        received: feed.received.load(Relaxed),
        queue_drops: feed.queue_drops.load(Relaxed),
        mqtt_connected: feed.mqtt_connected.load(Relaxed),
        mqtt_errors: feed.mqtt_errors.load(Relaxed),
        shared_memory_busy: feed.memory_busy.load(Relaxed),
    }
}

impl Drop for Integration {
    fn drop(&mut self) {
        self.feed.stop.store(true, Relaxed);
        for worker in &self.workers {
            worker.thread().unpark();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn dispatch(feed: &Feed, #[cfg(windows)] mut memory: Option<shared_memory::Mapping>) {
    loop {
        {
            let mut history = feed.history.lock().unwrap();
            for _ in 0..CAPACITY {
                let Some(frame) = feed.queue.pop() else {
                    break;
                };
                if history.len() == CAPACITY {
                    history.pop_front();
                }
                history.push_back(frame);
            }
        }
        if feed.stop.load(Relaxed) {
            break;
        }
        #[cfg(windows)]
        if let Some(memory) = &mut memory {
            memory.update(feed);
        }
        thread::park_timeout(IDLE);
    }
    #[cfg(windows)]
    if let Some(memory) = &mut memory {
        memory.close(feed);
    }
}
