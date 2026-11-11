//! Read-only Wayland color-management output queries. WSI owns the surface color
//! description; this separate connection never changes the user's output mode.
//! Missing protocol, output, HDR encoding or luminance info selects SDR.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    protocol::{wl_callback, wl_output, wl_registry},
};
use wayland_protocols::wp::color_management::v1::client::{
    wp_color_management_output_v1 as output, wp_color_manager_v1 as manager,
    wp_image_description_info_v1 as information, wp_image_description_v1 as description,
};

/// (HDR headroom relative to 203-nit content white, SDR white / scRGB 80 nits).
pub(super) fn display_range(output_name: &str) -> Option<(f32, f32)> {
    struct Worker {
        send: mpsc::SyncSender<String>,
        receive: mpsc::Receiver<(String, Option<(f32, f32)>)>,
        values: HashMap<String, (Instant, Option<(f32, f32)>)>,
        pending: Option<(String, Instant)>,
    }
    static WORKER: OnceLock<Mutex<Worker>> = OnceLock::new();
    let mut worker = WORKER
        .get_or_init(|| {
            let (send, requests) = mpsc::sync_channel::<String>(1);
            let (results, receive) = mpsc::channel();
            std::thread::spawn(move || {
                while let Ok(name) = requests.recv() {
                    let range = query(&name);
                    if results.send((name, range)).is_err() {
                        break;
                    }
                }
            });
            Mutex::new(Worker {
                send,
                receive,
                values: HashMap::new(),
                pending: None,
            })
        })
        .lock()
        .unwrap();
    while let Ok((name, value)) = worker.receive.try_recv() {
        worker.pending = None;
        worker.values.insert(name, (Instant::now(), value));
    }
    let stale = worker
        .values
        .get(output_name)
        .is_none_or(|(time, _)| time.elapsed() > Duration::from_secs(2));
    if stale && worker.pending.is_none() && worker.send.try_send(output_name.to_owned()).is_ok() {
        worker.pending = Some((output_name.to_owned(), Instant::now()));
    }
    // Do not keep old HDR capability after an unresponsive/reconfigured compositor.
    worker
        .values
        .get(output_name)
        .filter(|(time, _)| time.elapsed() < Duration::from_secs(4))
        .and_then(|(_, v)| *v)
}
#[derive(Default)]
struct Output {
    proxy: Option<wl_output::WlOutput>,
    name: Option<String>,
    tf: Option<u32>,
    maximum: Option<f32>,
    reference: Option<f32>,
    target: Option<f32>,
    done: bool,
}
#[derive(Default)]
struct State {
    manager: Option<manager::WpColorManagerV1>,
    outputs: HashMap<u32, Output>,
    sync_done: bool,
}
fn query(name: &str) -> Option<(f32, f32)> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let connection = Connection::connect_to_env().ok()?;
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    let mut state = State::default();
    let _registry = connection.display().get_registry(&handle, ());
    roundtrip(&connection, &mut queue, &mut state, deadline)?;
    roundtrip(&connection, &mut queue, &mut state, deadline)?;
    let manager = state.manager.clone()?;
    let (&id, source) = state
        .outputs
        .iter()
        .find(|(_, o)| o.name.as_deref() == Some(name))?;
    let output = manager.get_output(source.proxy.as_ref()?, &handle, ());
    let _description = output.get_image_description(&handle, id);
    for _ in 0..3 {
        roundtrip(&connection, &mut queue, &mut state, deadline)?;
        if state.outputs.get(&id)?.done {
            break;
        }
    }
    let result = state.outputs.get(&id)?;
    output.destroy();
    manager.destroy();
    if !result.done {
        return None;
    }
    range(
        result.tf?,
        result.maximum?,
        result.reference?,
        result.target,
    )
}
fn roundtrip(
    connection: &Connection,
    queue: &mut wayland_client::EventQueue<State>,
    state: &mut State,
    deadline: Instant,
) -> Option<()> {
    use std::os::fd::AsRawFd;
    state.sync_done = false;
    let _sync = connection.display().sync(&queue.handle(), ());
    while !state.sync_done {
        if Instant::now() >= deadline {
            return None;
        }
        queue.dispatch_pending(state).ok()?;
        if state.sync_done {
            break;
        }
        connection.flush().ok()?;
        if let Some(guard) = queue.prepare_read() {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let mut fd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one initialized pollfd lives through this bounded call.
            let ready = unsafe {
                libc::poll(
                    &mut fd,
                    1,
                    remaining.as_millis().min(i32::MAX as u128) as i32,
                )
            };
            if ready <= 0 {
                return None;
            }
            guard.read().ok()?;
        }
    }
    Some(())
}
impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.sync_done = true;
        }
    }
}

fn range(tf: u32, maximum: f32, reference: f32, target: Option<f32>) -> Option<(f32, f32)> {
    // Protocol transfer enum: ext_linear=5, ST2084 PQ=11, HLG=13. An ordinary
    // wide-gamut SDR profile must not be mistaken for HDR just because FP16 exists.
    if !matches!(tf, 5 | 11 | 13) || !reference.is_finite() || reference <= 0. {
        return None;
    }
    let peak = target.unwrap_or(maximum).min(maximum);
    if !peak.is_finite() || peak <= reference || peak <= 203. {
        return None;
    }
    Some(((peak / 203.).max(1.), (reference / 80.).clamp(0.1, 16.)))
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "wp_color_manager_v1" {
                state.manager = Some(registry.bind(name, version.min(3), q, ()));
            } else if interface == "wl_output" && version >= 4 {
                let proxy = registry.bind(name, 4, q, name);
                if state.outputs.len() >= 64 {
                    return;
                }
                state.outputs.insert(
                    name,
                    Output {
                        proxy: Some(proxy),
                        ..Default::default()
                    },
                );
            }
        }
    }
}
impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(output) = state.outputs.get_mut(id) {
                output.name = Some(name);
            }
        }
    }
}
impl Dispatch<description::WpImageDescriptionV1, u32> for State {
    fn event(
        state: &mut Self,
        description: &description::WpImageDescriptionV1,
        event: description::Event,
        id: &u32,
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        match event {
            description::Event::Ready { .. } | description::Event::Ready2 { .. } => {
                description.get_information(q, *id);
            }
            description::Event::Failed { .. } => {
                if let Some(output) = state.outputs.get_mut(id) {
                    output.done = true;
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<information::WpImageDescriptionInfoV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &information::WpImageDescriptionInfoV1,
        event: information::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.get_mut(id) else {
            return;
        };
        match event {
            information::Event::TfNamed { tf } => {
                output.tf = Some(match tf {
                    WEnum::Value(v) => v as u32,
                    WEnum::Unknown(v) => v,
                })
            }
            information::Event::Luminances {
                max_lum,
                reference_lum,
                ..
            } => {
                output.maximum = Some(max_lum as f32);
                output.reference = Some(reference_lum as f32);
            }
            information::Event::TargetLuminance { max_lum, .. } => {
                output.target = Some(max_lum as f32)
            }
            information::Event::Done => output.done = true,
            _ => {}
        }
    }
}
wayland_client::delegate_noop!(State: ignore manager::WpColorManagerV1);
wayland_client::delegate_noop!(State: ignore output::WpColorManagementOutputV1);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unresponsive_compositor_query_returns_at_deadline() {
        let (client, _server) = std::os::unix::net::UnixStream::pair().unwrap();
        let connection = Connection::from_socket(client).unwrap();
        let mut queue = connection.new_event_queue();
        let start = Instant::now();
        assert!(
            roundtrip(
                &connection,
                &mut queue,
                &mut State::default(),
                start + Duration::from_millis(20)
            )
            .is_none()
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn missing_or_sdr_description_never_enables_hdr() {
        assert_eq!(range(9, 300., 203., Some(300.)), None);
        assert_eq!(range(5, 203., 203., None), None);
        assert_eq!(range(11, 10000., 203., Some(100.)), None);
    }
    #[test]
    fn hdr_target_luminance_controls_headroom_not_pq_encoding_maximum() {
        let (h, w) = range(11, 10000., 203., Some(1000.)).unwrap();
        assert!((h - 1000. / 203.).abs() < 1e-5);
        assert!((w - 203. / 80.).abs() < 1e-5);
    }
}
