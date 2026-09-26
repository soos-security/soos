//! IPC Camera Manager proxying live preview frames from `soos-daemon` over Unix domain socket.

use arc_swap::ArcSwapOption;
use soos_camera_v4l::{CameraManager, Frame, PixelFormat};
use soos_protocol::codec::{decode_preview, encode};
use soos_protocol::types::{
    PreviewResponse, Request, RequestKind, CURRENT_VERSION, MAX_PREVIEW_MESSAGE_SIZE,
};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Camera manager implementation querying real-time video frames from `soos-daemon`
/// via the `/run/soos/daemon.sock` IPC socket.
pub struct IpcCameraManager {
    latest_frame: Arc<ArcSwapOption<Frame>>,
    is_ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    worker_handle: Option<JoinHandle<()>>,
}

impl IpcCameraManager {
    /// Spawns a background thread streaming preview frames from the specified Unix domain socket path.
    pub fn spawn<P: AsRef<Path>>(socket_path: P) -> Self {
        let path = socket_path.as_ref().to_path_buf();
        let latest_frame = Arc::new(ArcSwapOption::empty());
        let is_ready = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));

        let latest_clone = Arc::clone(&latest_frame);
        let ready_clone = Arc::clone(&is_ready);
        let running_clone = Arc::clone(&running);

        let handle = thread::Builder::new()
            .name("soos-gui-ipc-cam".to_string())
            .spawn(move || {
                run_ipc_camera_worker(path, latest_clone, ready_clone, running_clone);
            })
            .ok();

        Self {
            latest_frame,
            is_ready,
            running,
            worker_handle: handle,
        }
    }

    /// Spawns an IPC camera manager pointing to the default daemon socket `/run/soos/daemon.sock`.
    pub fn spawn_default() -> Self {
        Self::spawn("/run/soos/daemon.sock")
    }

    /// Probes whether the daemon socket is alive and responsive to preview queries.
    pub fn probe<P: AsRef<Path>>(socket_path: P) -> bool {
        let Ok(mut stream) = UnixStream::connect(socket_path) else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));

        let req = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::PreviewFrame,
            request_id: [0xAA; 32],
            uid_hint: nix::unistd::getuid().as_raw(),
            service: "soos-gui".to_string(),
            deadline_monotonic_ns: u64::MAX,
        };

        let Ok(framed) = encode(&req) else {
            return false;
        };
        if stream.write_all(&framed).is_err() || stream.flush().is_err() {
            return false;
        }

        let mut len_bytes = [0u8; 4];
        if stream.read_exact(&mut len_bytes).is_err() {
            return false;
        }

        true
    }
}

impl CameraManager for IpcCameraManager {
    fn latest_frame(&self) -> Option<Arc<Frame>> {
        if !self.is_ready() {
            return None;
        }
        self.latest_frame.load_full()
    }

    fn is_ready(&self) -> bool {
        self.is_ready.load(Ordering::Acquire)
    }

    fn notify_activity(&self) {
        // Querying preview frames continuously wakes daemon camera activity on demand.
    }

    fn stop(&self) {
        self.running.store(false, Ordering::Release);
        self.is_ready.store(false, Ordering::Release);
    }
}

impl Drop for IpcCameraManager {
    fn drop(&mut self) {
        self.stop();
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

fn run_ipc_camera_worker(
    socket_path: PathBuf,
    latest_frame: Arc<ArcSwapOption<Frame>>,
    is_ready: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
) {
    let uid = nix::unistd::getuid().as_raw();

    while running.load(Ordering::Acquire) {
        let stream_result = UnixStream::connect(&socket_path);
        let mut stream = match stream_result {
            Ok(s) => s,
            Err(_) => {
                is_ready.store(false, Ordering::Release);
                thread::sleep(Duration::from_millis(200));
                continue;
            }
        };

        let _ = stream.set_read_timeout(Some(Duration::from_millis(1000)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));

        while running.load(Ordering::Acquire) {
            let req = Request {
                version: CURRENT_VERSION,
                kind: RequestKind::PreviewFrame,
                request_id: [0x5A; 32],
                uid_hint: uid,
                service: "soos-gui".to_string(),
                deadline_monotonic_ns: u64::MAX,
            };

            let Ok(encoded_req) = encode(&req) else {
                break;
            };

            if stream.write_all(&encoded_req).is_err() || stream.flush().is_err() {
                is_ready.store(false, Ordering::Release);
                break;
            }

            let mut len_bytes = [0u8; 4];
            if stream.read_exact(&mut len_bytes).is_err() {
                is_ready.store(false, Ordering::Release);
                break;
            }

            let Ok(declared_size) = usize::try_from(u32::from_be_bytes(len_bytes)) else {
                is_ready.store(false, Ordering::Release);
                break;
            };

            if declared_size > MAX_PREVIEW_MESSAGE_SIZE || declared_size == 0 {
                is_ready.store(false, Ordering::Release);
                break;
            }

            let total_size = declared_size.saturating_add(4);
            let mut buf = vec![0u8; total_size];
            if let Some(prefix) = buf.get_mut(..4) {
                prefix.copy_from_slice(&len_bytes);
            }

            let read_payload_failed = match buf.get_mut(4..) {
                Some(payload_slot) => stream.read_exact(payload_slot).is_err(),
                None => true,
            };
            if read_payload_failed {
                is_ready.store(false, Ordering::Release);
                break;
            }

            match decode_preview::<PreviewResponse>(&buf) {
                Ok(resp) => {
                    if resp.width > 0 && resp.height > 0 && !resp.data.is_empty() {
                        let format = match resp.format {
                            0 => PixelFormat::Rgb24,
                            1 => PixelFormat::Grey,
                            2 => PixelFormat::Yuyv,
                            3 => PixelFormat::Nv12,
                            4 => PixelFormat::Mjpeg,
                            _ => PixelFormat::Rgb24,
                        };

                        let frame = Frame::new(
                            resp.data,
                            resp.width,
                            resp.height,
                            resp.timestamp_monotonic_ns,
                            format,
                            resp.sequence,
                        );

                        latest_frame.store(Some(Arc::new(frame)));
                        is_ready.store(true, Ordering::Release);
                    }
                }
                Err(_) => {
                    is_ready.store(false, Ordering::Release);
                    break;
                }
            }

            // Target ~30 FPS preview query rate
            thread::sleep(Duration::from_millis(33));
        }

        is_ready.store(false, Ordering::Release);
        thread::sleep(Duration::from_millis(100));
    }
}
