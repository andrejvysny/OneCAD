use onecad_protocol::framing::{encode_frame, read_frame_blocking};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::{
    fs::{self, File},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};

pub fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub struct Worker {
    child: Child,
    input: ChildStdin,
    frames: Receiver<(Vec<u8>, Value)>,
    evidence: File,
    pub hello: Value,
    pub transcript: Vec<Value>,
}

impl Worker {
    pub fn start(binary: &Path, directory: &Path) -> Self {
        fs::create_dir_all(directory).unwrap();
        let mut child = Command::new(binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(File::create(directory.join("stderr.log")).unwrap())
            .spawn()
            .expect("production worker starts");
        let input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (send, frames) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(Some(frame)) = read_frame_blocking(&mut output) {
                let json = serde_json::from_slice(&frame.json).unwrap();
                let bytes = encode_frame(&frame.json, &frame.bin).unwrap();
                if send.send((bytes, json)).is_err() {
                    break;
                }
            }
        });
        let mut worker = Self {
            child,
            input,
            frames,
            evidence: File::create(directory.join("frames.ndjson")).unwrap(),
            hello: Value::Null,
            transcript: vec![],
        };
        worker.hello = worker.read();
        assert_eq!(worker.hello["t"], "hello");
        assert_eq!(worker.hello["result"]["protocolVersion"], 1);
        assert_eq!(worker.hello["result"]["occt"]["version"], "8.0.1");
        worker
    }

    fn read(&mut self) -> Value {
        let (bytes, json) = self
            .frames
            .recv_timeout(Duration::from_secs(30))
            .expect("worker emits checked OCW1 frame within 30 seconds");
        self.record(serde_json::json!({
            "direction": "receive", "sha256": sha(&bytes),
            "ocw1Hex": hex(&bytes), "envelope": json,
        }));
        json
    }

    pub fn request(&mut self, bytes: &[u8], id: u64) -> Value {
        self.record(serde_json::json!({
            "direction": "send", "sha256": sha(bytes), "ocw1Hex": hex(bytes),
            "envelope": serde_json::from_slice::<Value>(&bytes[12..]).unwrap(),
        }));
        self.input.write_all(bytes).unwrap();
        self.input.flush().unwrap();
        loop {
            let response = self.read();
            if response["t"] == "resp" && response["id"] == id {
                assert_eq!(response["ok"], true, "{response}");
                return response["result"].clone();
            }
        }
    }

    fn record(&mut self, frame: Value) {
        serde_json::to_writer(&mut self.evidence, &frame).unwrap();
        self.evidence.write_all(b"\n").unwrap();
        self.evidence.flush().unwrap();
        self.transcript.push(frame);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
