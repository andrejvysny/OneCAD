//! GPU pass timing with timestamp queries at pass boundaries (Apple GPUs sample only at stage
//! boundaries, so there are no timestamps inside a pass).

pub const MAX_PASSES: u32 = 16;

pub struct Timer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    period_ns: f64,
    pub labels: Vec<&'static str>,
}

impl Timer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Timer {
        let n = MAX_PASSES * 2;
        let size = n as u64 * 8;
        Timer {
            set: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("passes"), ty: wgpu::QueryType::Timestamp, count: n }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ts_resolve"),
                size,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ts_read"),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            period_ns: queue.get_timestamp_period() as f64,
            labels: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.labels.clear();
    }

    /// The timestamp writes for the next pass, named `label`.
    pub fn pass(&mut self, label: &'static str) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let k = self.labels.len() as u32;
        if k >= MAX_PASSES {
            return None;
        }
        self.labels.push(label);
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(2 * k),
            end_of_pass_write_index: Some(2 * k + 1),
        })
    }

    pub fn finish(&self, enc: &mut wgpu::CommandEncoder) {
        let n = self.labels.len() as u32 * 2;
        if n == 0 {
            return;
        }
        enc.resolve_query_set(&self.set, 0..n, &self.resolve, 0);
        enc.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, n as u64 * 8);
    }

    /// Milliseconds per pass (end − begin, signed: a negative value is a bad sample), after the
    /// submission that carried `finish` has completed.
    pub fn read(&self, device: &wgpu::Device) -> Vec<(&'static str, f64)> {
        let n = self.labels.len() as u64 * 2;
        if n == 0 {
            return Vec::new();
        }
        let slice = self.read.slice(0..n * 8);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let ticks: Vec<u64> = match slice.get_mapped_range() {
            Ok(v) => v.chunks_exact(8).map(|c| u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])).collect(),
            Err(_) => vec![0; n as usize],
        };
        self.read.unmap();
        if std::env::var_os("RENDER_LOOK_TS_DEBUG").is_some() {
            eprintln!("ticks {:?} period {}", ticks, self.period_ns);
        }
        self.labels
            .iter()
            .enumerate()
            .map(|(k, l)| (*l, (ticks[2 * k + 1] as i64 - ticks[2 * k] as i64) as f64 * self.period_ns / 1e6))
            .collect()
    }
}
