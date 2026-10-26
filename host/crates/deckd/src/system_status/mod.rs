//! Local computer telemetry. One lightweight sampler, no subprocesses or admin
//! privileges; transient readings go through the normal coalescing P3 lane.
use crate::core::handle::{CoreHandle, CoreMsg};
use deck_proto::SystemStatus;
use std::time::Duration;

#[cfg(target_os = "macos")]
mod macos;

#[derive(Default)]
pub struct Sampler {
    previous_cpu: Option<[u32; 4]>,
    sequence: u32,
}
fn cpu_percent(previous: [u32; 4], current: [u32; 4]) -> Option<u8> {
    let delta = std::array::from_fn::<_, 4, _>(|i| u64::from(current[i].wrapping_sub(previous[i])));
    let total: u64 = delta.iter().sum();
    // Mach CPU_STATE_IDLE is slot 2; normalize across all cores to 0..100.
    (total > 0).then(|| ((total - delta[2]) * 100 / total).min(100) as u8)
}
fn mib(bytes: u64) -> u32 {
    (bytes / 1_048_576).min(u64::from(u32::MAX)) as u32
}
impl Sampler {
    pub fn sample(&mut self) -> SystemStatus {
        self.sequence = self.sequence.wrapping_add(1);
        let mut status = SystemStatus {
            sample: self.sequence,
            ..SystemStatus::EMPTY
        };
        #[cfg(target_os = "macos")]
        {
            let cpu = macos::cpu_ticks();
            status.cpu = self
                .previous_cpu
                .zip(cpu)
                .and_then(|(old, new)| cpu_percent(old, new));
            self.previous_cpu = cpu;
            if let Some((used, total)) = macos::memory() {
                status.memory_used_mib = mib(used.min(total));
                status.memory_total_mib = mib(total);
            }
            if let Some((free, total)) = macos::disk() {
                status.disk_free_mib = mib(free.min(total));
                status.disk_total_mib = mib(total);
            }
            status.gpu = macos::gpu();
        }
        status
    }
}

pub async fn run(core: CoreHandle, stop: tokio::sync::watch::Receiver<bool>) {
    // Native OS calls stay off both the reducer and async I/O threads.
    let _ = tokio::task::spawn_blocking(move || {
        let mut sampler = Sampler::default();
        while !*stop.borrow() && stop.has_changed().is_ok() {
            if !core.send_blocking(CoreMsg::SystemStatus(sampler.sample())) {
                break;
            }
            for _ in 0..20 {
                if *stop.borrow() || stop.has_changed().is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_load_is_a_delta_normalized_across_cores() {
        assert_eq!(cpu_percent([0; 4], [20, 10, 70, 0]), Some(30));
        assert_eq!(
            cpu_percent([100, 100, 100, 0], [140, 120, 240, 0]),
            Some(30)
        );
        assert_eq!(cpu_percent([1; 4], [1; 4]), None);
        assert_eq!(
            cpu_percent([u32::MAX - 9, 0, 0, 0], [10, 0, 80, 0]),
            Some(20)
        );
    }
}
