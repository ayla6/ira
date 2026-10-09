mod devices;
mod inputs;
mod key_intercept;
mod output_stack;
mod overlay_nav;
mod overlay_toggle;

// Device helpers owned by the hub on the physical side.
pub(crate) use devices::{
    apply_controller_layout, open_rumble, open_sensor, probe_sensor, reconnect_gamepad,
    resolved_layout_for, GyroSource,
};
pub(crate) use sensor::now_us;
mod loop_io;
mod outputs;
mod sensor;
mod setup;

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::InputSource;

use super::hub::{HubCommand, HubHandle, PadEvent};
use super::{Arguments, SessionEvent, SteamWatcher};
use super::signals::{install_signal_handlers, STOP_REQUESTED};
use devices::{apply_controller_deadzone, make_gyro_processor};
use inputs::{poll_cursor_switch, process_pad_events};
use loop_io::{
    floor_poll_timeout, FOCUS_POLL_INTERVAL, LoopActivity, PROCESS_POLL_INTERVAL,
    PROFILE_POLL_INTERVAL,
};
use outputs::{
    attach_motion_outputs, emit_mapped, emit_outputs, rebuild_output_devices, reload_profile,
    stop_child, LiveOutputs, OutputTargets,
};
use sensor::{forward_samples, process_tick, service_twin_events, tick_needed_for};
use setup::{mirror_identity, setup_session, SessionSetup};

/// Park ceiling when no scheduled work needs a shorter deadline: pad events
/// wake the channel instantly, so a long park only delays idle bookkeeping.
const IDLE_PARK: Duration = Duration::from_secs(1);

pub fn run_session(arguments: Arguments) -> Result<i32, String> {
    STOP_REQUESTED.store(false, Ordering::Relaxed);
    install_signal_handlers();
    let SessionSetup {
        mut trace,
        hub,
        pad_events,
        mut mapper,
        mut profile_monitor,
        native_passthrough,
        mut keyboard,
        mut mouse,
        mut virtual_gamepad,
        mut pad_state,
        mut pipeline,
        motion_enabled,
        mut pad_vendor,
        mut pad_product,
        mut mirror,
        mut tick_needed,
        mut report_rate,
        mut steam_watch,
        mut launcher_exit_code,
        mut child,
        mut schedule,
        focus,
        mut paused_for_focus,
        cursor_watcher,
        mut cursor_visible,
    } = setup_session(&arguments)?;
    // Profile saves wake the loop through the pad channel; the fd gates the
    // fallback cadence when inotify could not be created.
    let profile_wake_fd: Option<libc::c_int> =
        profile_monitor.as_ref().and_then(|monitor| monitor.fd());
    // The session unsubscribes from the pad hub on every exit path.
    let _unsubscribe = UnsubscribeOnDrop(&hub, arguments.session_id);
    // The hub routes pad traffic to whoever owns focus; until it says
    // otherwise this session is frozen. `None` means the first focus
    // reading has not been reported yet.
    let mut reported_focus: Option<bool> = if focus.is_none() {
        Some(true)
    } else {
        None
    };
    let mut hub_live = false;
    let mut pending: Vec<PadEvent> = Vec::new();
    // Overlay toggle authority (gamescope/flatpak only): the daemon owns the physical
    // pad, the one input path the sandboxed game cannot hide from.
    let mut overlay_toggle = overlay_toggle::OverlayToggle::arm(arguments.env.as_deref());
    if overlay_toggle.is_some() {
        eprintln!("ira-input: overlay toggle chord armed for session");
    }
    // Keyboard chord authority + interception (same marker): grabbed and re-emitted
    // while focused, consuming only the chord; unfocused it only snoops.
    let mut key_intercept = key_intercept::KeyIntercept::arm(arguments.env.as_deref());
    if key_intercept.is_some() {
        eprintln!("ira-input: overlay keyboard intercept armed for session");
        key_intercept::KeyIntercept::log_input_census();
    }
    // Last pass's overlay visibility, for the rising-edge release below.
    let mut overlay_was_open = false;
    // Buttons consumed by the overlay never reached the game, so on close their
    // releases are swallowed too, or a held Guide/Enter leaks a phantom click.
    let mut consumed_while_open: HashSet<InputSource> = HashSet::new();
    // Close-edge swallow set: consumed buttons held at close, dropped at each
    // release; innocent held-through buttons still hand back below.
    let mut swallowed: HashSet<InputSource> = HashSet::new();
    // Overlay gamepad navigation arms lazily: the host canvas appears after the
    // session starts, so early arms fail and are retried on cadence while visible.
    let mut overlay_nav: Option<overlay_nav::OverlayNav> = None;
    let mut nav_retry_at = Instant::now();
    // Motion samples queued since the last pass; every sample is processed, or a
    // dropped one stretches the window and turns micro-motion into phantom rotation.
    let mut samples: Vec<crate::SensorSample> = Vec::new();
    // Whether any sample reached `forward_samples` since the last tick ran;
    // the tick only refreshes the twins when the sample stream was silent.
    let mut samples_since_tick = false;

    loop {
        let tick_interval = report_rate.interval();
        let run_tick = tick_needed && schedule.sensor.elapsed() >= tick_interval;
        if run_tick {
            schedule.sensor = Instant::now();
        }
        if focus.is_some() && schedule.focus.elapsed() >= FOCUS_POLL_INTERVAL {
            schedule.focus = Instant::now();
            let focused = focus
                .as_ref()
                .is_some_and(crate::FocusWatcher::game_is_focused);
            // Every change from the last reported state goes to the hub, including
            // the first reading claiming the pad for a focused-at-launch game.
            if reported_focus != Some(focused) {
                reported_focus = Some(focused);
                hub.send(HubCommand::Focus {
                    id: arguments.session_id,
                    focused,
                });
            }
            if !focused && !paused_for_focus {
                paused_for_focus = true;
                eprintln!("ira-input: game window unfocused; pausing input");
                if let Err(error) = release_outputs(
                    &mut mapper, &mut virtual_gamepad, &mut keyboard, &mut mouse,
                    &mut pad_state, &mut trace,
                ) {
                    eprintln!("ira-input: failed to release outputs on pause: {error}");
                }
            } else if focused && paused_for_focus {
                paused_for_focus = false;
                eprintln!("ira-input: game window focused; resuming input");
            }
        }
        if let Some(watcher) = cursor_watcher.as_ref() {
            if schedule.cursor.elapsed() >= FOCUS_POLL_INTERVAL {
                schedule.cursor = Instant::now();
                let targets = OutputTargets {
                    gamepad: &mut virtual_gamepad,
                    keyboard: keyboard.as_mut(),
                    mouse: mouse.as_mut(),
                    pad: &mut pad_state,
                };
                if let Err(error) = poll_cursor_switch(
                    watcher,
                    &mut cursor_visible,
                    &mut mapper,
                    targets,
                    &mut trace,
                ) {
                    eprintln!("ira-input: cursor set switching failed: {error}");
                }
            }
        }
        // Keep every uhid twin's kernel conversation moving regardless of pause or
        // tick gating, or handshakes and rumble replay stall.
        service_twin_events(&mut pipeline);

        // Consume everything the hub sent: control events act immediately,
        // input and samples queue for the mapping pass below.
        loop {
            match pad_events.try_recv() {
                Ok(event) => pending.push(event),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
        for event in std::mem::take(&mut pending) {
            match event {
                PadEvent::Input(_) | PadEvent::Sample(_) => pending.push(event),
                PadEvent::Motion(motion) => {
                    pipeline.motion_available = motion;
                    pipeline.ever_had_sensor |= motion;
                    if motion {
                        // A late motion source needs outputs attached now: a motion-less
                        // stack carries no motion node, twins, or cemuhook stream.
                        let mut outputs = LiveOutputs {
                            mapper: &mut mapper,
                            gamepad: &mut virtual_gamepad,
                            keyboard: &mut keyboard,
                            mouse: &mut mouse,
                            pad: &mut pad_state,
                            pipeline: &mut pipeline,
                            motion_enabled,
                            mirror: mirror.clone(),
                            native_passthrough,
                        };
                        if attach_motion_outputs(&mut outputs, &mut trace) {
                            eprintln!("ira-input: motion outputs attached");
                        }
                    }
                    tick_needed = tick_needed_for(&pipeline, &mapper);
                }
                PadEvent::Connected {
                    name,
                    path,
                    vendor,
                    product,
                    version,
                } => {
                    pad_vendor = vendor;
                    pad_product = product;
                    eprintln!(
                        "ira-input: controller identity {vendor:04x}:{product:04x}; \
                         applying per-controller calibration and deadzones"
                    );
                    pipeline.gyro_processor = make_gyro_processor(
                        mapper.profile(),
                        vendor,
                        product,
                        arguments.calibration.as_deref(),
                    );
                    apply_controller_deadzone(
                        &mut mapper,
                        arguments.calibration.as_deref(),
                        vendor,
                        product,
                    );
                    // A new physical identity means new glyphs: rebuild the virtual gamepad
                    // to mirror it; same-pad reconnects compare equal and skip the swap.
                    let connected_mirror = mirror_identity(&Some((
                        name.clone(),
                        path.clone(),
                        vendor,
                        product,
                        version,
                    )));
                    if connected_mirror != mirror {
                        mirror = connected_mirror;
                        let mut outputs = LiveOutputs {
                            mapper: &mut mapper,
                            gamepad: &mut virtual_gamepad,
                            keyboard: &mut keyboard,
                            mouse: &mut mouse,
                            pad: &mut pad_state,
                            pipeline: &mut pipeline,
                            motion_enabled,
                            mirror: mirror.clone(),
                            native_passthrough,
                        };
                        rebuild_output_devices(&mut outputs, &mut trace);
                        eprintln!(
                            "ira-input: virtual gamepad now mirrors {vendor:04x}:{product:04x}"
                        );
                    }
                    pipeline.last_sensor_us = None;
                    report_rate.reset();
                    // Footer glyphs follow the physical pad, not the
                    // virtual backend.
                    if let Some(toggle) = overlay_toggle.as_ref() {
                        toggle.publish_family(crate::family_for(vendor, &name));
                    }
                    // Fresh physical state: buttons held while (re)connecting
                    // are grip, not toggle intent (settle window).
                    if let Some(toggle) = overlay_toggle.as_mut() {
                        toggle.note_connect();
                    }
                    if let Some(events) = &arguments.events {
                        let _ = events.send(SessionEvent::Controller {
                            connected: true,
                            name,
                            path,
                        });
                    }
                }
                PadEvent::ProfileChanged => {}
                PadEvent::ReloadProfile(path) => {
                    // Point the watcher at the new file; the regular reload path applies
                    // it (mapping edits hot-apply, kind changes rebuild the stack).
                    if !devices::is_real_profile(&path) {
                        eprintln!(
                            "ira-input: layout switch ignored, {} is not a profile",
                            path.display()
                        );
                    } else if let Some(monitor) = profile_monitor.as_mut() {
                        eprintln!("ira-input: switching layout to {}", path.display());
                        monitor.retarget(&path);
                    } else {
                        eprintln!(
                            "ira-input: layout switch to {} ignored; \
                             the session runs the builtin layout",
                            path.display()
                        );
                    }
                }
                PadEvent::Live { pressed } => {
                    hub_live = true;
                    // Reconcile across the routing gap: mapper-held but pad-up lost its
                    // release, so emit real releases instead of sticking forever.
                    let missing = inputs::releases_for_missing(
                        mapper.held_buttons(),
                        &pressed.into_iter().collect(),
                        now_us(),
                    );
                    if !missing.is_empty() {
                        eprintln!(
                            "ira-input: session {} reconciled {} stuck output(s) on resume",
                            arguments.session_id,
                            missing.len()
                        );
                    }
                    for event in missing {
                        if let Err(error) = emit_mapped(
                            &mut mapper,
                            OutputTargets {
                                gamepad: &mut virtual_gamepad,
                                keyboard: keyboard.as_mut(),
                                mouse: mouse.as_mut(),
                                pad: &mut pad_state,
                            },
                            event,
                            &mut trace,
                        ) {
                            eprintln!("ira-input: failed to emit reconciled release: {error}");
                        }
                    }
                }
                PadEvent::Frozen => {
                    eprintln!("ira-input: session {} frozen", arguments.session_id);
                    if hub_live {
                        hub_live = false;
                        if let Err(error) = release_outputs(
                            &mut mapper, &mut virtual_gamepad, &mut keyboard, &mut mouse,
                            &mut pad_state, &mut trace,
                        ) {
                            eprintln!("ira-input: failed to release frozen outputs: {error}");
                        }
                    }
                }
                PadEvent::Stop => {
                    // The server replaced this session: wind down with no exit code.
                    eprintln!("ira-input: session {} stopping on request", arguments.session_id);
                    if let Err(error) = release_outputs(
                        &mut mapper, &mut virtual_gamepad, &mut keyboard, &mut mouse,
                        &mut pad_state, &mut trace,
                    ) {
                        eprintln!("ira-input: failed to release outputs on stop: {error}");
                    }
                    return Ok(0);
                }
            }
        }

        let unfocused_or_dead = paused_for_focus || !hub_live;
        // Overlay toggle pre-pass: chord completions are swallowed before anything
        // else sees them; runs even when paused so the chord always reaches the overlay.
        if let Some(toggle) = overlay_toggle.as_mut() {
            let mut fired = false;
            pending.retain_mut(|event| {
                let PadEvent::Input(input) = event else {
                    return true;
                };
                match toggle.filter(input) {
                    overlay_toggle::ToggleAction::Forward => true,
                    overlay_toggle::ToggleAction::Swallow => false,
                    overlay_toggle::ToggleAction::Fire => {
                        fired = true;
                        // The completing release is consumed here; its press was already
                        // swallowed when it armed the pending chord.
                        consumed_while_open.insert(input.source);
                        false
                    }
                    overlay_toggle::ToggleAction::FireScreenshot => {
                        if toggle.push_screenshot() {
                            consumed_while_open.insert(input.source);
                        }
                        false
                    }
                    overlay_toggle::ToggleAction::FireRecord => {
                        if toggle.push_record_toggle() {
                            consumed_while_open.insert(input.source);
                        }
                        false
                    }
                }
            });
            if fired {
                match toggle.flip() {
                    Some(visible) => {
                        eprintln!("ira-input: overlay toggle flipped overlay to {visible}")
                    }
                    None => eprintln!("ira-input: overlay toggle debounced"),
                }
            }
        }
        // Keyboard chord intercept, focus-gated; the previous pass's visibility
        // switches forwarding to consuming.
        if let Some(intercept) = key_intercept.as_mut() {
            if intercept.poll(!paused_for_focus, overlay_was_open) {
                match intercept.flip() {
                    Some(visible) => {
                        eprintln!("ira-input: key intercept flipped overlay to {visible}")
                    }
                    None => eprintln!("ira-input: key intercept debounced"),
                }
            }
        }
        // While the overlay is up the game gets no pad input; the rising edge
        // releases everything held so nothing sticks.
        let overlay_open = overlay_toggle
            .as_ref()
            .is_some_and(|toggle| toggle.is_visible());
        if overlay_open && !overlay_was_open {
            if let Err(error) = release_outputs(
                &mut mapper,
                &mut virtual_gamepad,
                &mut keyboard,
                &mut mouse,
                &mut pad_state,
                &mut trace,
            ) {
                eprintln!("ira-input: failed to release outputs for overlay: {error}");
            }
        }
        // Falling edge: consumed buttons swallow until release, innocent held-through
        // buttons hand back; sticks self-heal on the next deflection.
        if !overlay_open && overlay_was_open {
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            hub.send(HubCommand::GetPressed { reply: reply_tx });
            if let Ok(pressed) = reply_rx.recv_timeout(Duration::from_millis(200)) {
                swallowed = pressed
                    .iter()
                    .filter(|source| {
                        matches!(source, InputSource::Button(_))
                            && consumed_while_open.contains(*source)
                    })
                    .copied()
                    .collect();
                if !swallowed.is_empty() {
                    eprintln!(
                        "ira-input: swallowing {} held button(s) on close until release",
                        swallowed.len()
                    );
                }
                let handed: HashSet<InputSource> =
                    pressed.iter().copied().collect::<HashSet<_>>()
                        .difference(&swallowed)
                        .copied()
                        .collect();
                let mut handed_back = 0;
                for event in inputs::presses_for_held(
                    mapper.held_buttons(),
                    &handed,
                    now_us(),
                ) {
                    handed_back += 1;
                    if let Err(error) = emit_mapped(
                        &mut mapper,
                        OutputTargets {
                            gamepad: &mut virtual_gamepad,
                            keyboard: keyboard.as_mut(),
                            mouse: mouse.as_mut(),
                            pad: &mut pad_state,
                        },
                        event,
                        &mut trace,
                    ) {
                        eprintln!("ira-input: failed to emit hand-back press: {error}");
                    }
                }
                if handed_back > 0 {
                    eprintln!("ira-input: handed back {handed_back} held button(s) on close");
                }
            }
            consumed_while_open.clear();
        }
        overlay_was_open = overlay_open;
        // Swallow-upon-release: consumed buttons never reach the game until each
        // reports up; runs open or closed so blackout releases still retire.
        if !swallowed.is_empty() {
            pending.retain(|event| {
                let PadEvent::Input(input) = event else {
                    return true;
                };
                if !swallowed.contains(&input.source) {
                    return true;
                }
                if inputs::is_release(&input.source, input.value) {
                    swallowed.remove(&input.source);
                }
                false
            });
        }
        // Overlay navigation (gamescope/flatpak only): the daemon translates pad
        // presses to canvas commands while visible; the game never sees them.
        if overlay_open {
            if overlay_nav.is_none()
                && nav_retry_at.elapsed() >= Duration::from_secs(1)
            {
                nav_retry_at = Instant::now();
                overlay_nav = overlay_nav::OverlayNav::arm(arguments.env.as_deref());
                if overlay_nav.is_some() {
                    eprintln!("ira-input: overlay navigation armed for session");
                }
            }
            if let Some(nav) = overlay_nav.as_mut() {
                for event in pending.iter() {
                    if let PadEvent::Input(input) = event {
                        // Any pad press while open is consumed: holding it through the
                        // close must swallow, never hand back.
                        if matches!(input.source, InputSource::Button(_)) && input.value > 0.5
                        {
                            consumed_while_open.insert(input.source);
                        }
                        nav.feed(input);
                    }
                }
                // Held directions auto-repeat like Big Picture.
                nav.poll_repeat();
            }
        }
        let paused = unfocused_or_dead || overlay_open;
        let result = if paused {
            // Frozen or unfocused: the hub forwards no inputs, and anything
            // already queued is stale — drop it along with motion samples.
            pending.clear();
            samples.clear();
            Ok(())
        } else {
            process_pad_events(
                &mut pending,
                &mut samples,
                &mut mapper,
                &mut report_rate,
                OutputTargets {
                    gamepad: &mut virtual_gamepad,
                    keyboard: keyboard.as_mut(),
                    mouse: mouse.as_mut(),
                    pad: &mut pad_state,
                },
                &mut trace,
            )
            .and_then(|()| {
                // The passthrough runs per pass, sample-driven; only the
                // mapping engine's continuous outputs stay on the tick.
                samples_since_tick |= forward_samples(
                    &mut pipeline,
                    &mut mapper,
                    &OutputTargets {
                        gamepad: &mut virtual_gamepad,
                        keyboard: keyboard.as_mut(),
                        mouse: mouse.as_mut(),
                        pad: &mut pad_state,
                    },
                    &mut trace,
                    std::mem::take(&mut samples),
                );
                process_tick(
                    &mut pipeline,
                    &mut mapper,
                    OutputTargets {
                        gamepad: &mut virtual_gamepad,
                        keyboard: keyboard.as_mut(),
                        mouse: mouse.as_mut(),
                        pad: &mut pad_state,
                    },
                    &mut trace,
                    run_tick,
                    samples_since_tick,
                )
                .map(|()| {
                    if run_tick {
                        samples_since_tick = false;
                    }
                })
            })
        };
        trace.flush();
        // Rumble uploads are drained every pass so the kernel queue never
        // overflows; while frozen or paused the commands simply go nowhere.
        if !paused && mapper.profile().rumble {
            for command in virtual_gamepad.poll_rumble() {
                hub.send(HubCommand::Rumble {
                    id: arguments.session_id,
                    command,
                });
            }
            // Native transports carry rumble inside HID output reports
            // instead (the uinput pad does not exist then).
            let hid_rumble = if let Some(device) = pipeline.switch_pro_hid.as_mut() {
                device.take_rumble()
            } else if let Some(device) = pipeline.ds4_hid.as_mut() {
                device.take_rumble()
            } else if let Some(device) = pipeline.dualsense_hid.as_mut() {
                device.take_rumble()
            } else {
                Vec::new()
            };
            for command in hid_rumble {
                hub.send(HubCommand::Rumble {
                    id: arguments.session_id,
                    command,
                });
            }
        } else {
            virtual_gamepad.poll_rumble();
        }
        if let Err(error) = result {
            if let Err(reset_error) = release_outputs(
                &mut mapper, &mut virtual_gamepad, &mut keyboard, &mut mouse,
                &mut pad_state, &mut trace,
            ) {
                eprintln!("ira-input: failed to emit reset releases: {reset_error}");
            }
            stop_child(&mut child);
            return Err(error);
        }
        if STOP_REQUESTED.load(Ordering::Relaxed) {
            if let Some(watcher) = steam_watch.as_mut() {
                watcher.request_stop_and_join();
            }
            stop_child(&mut child);
            return Ok(130);
        }
        let child_status = if child.is_some() && schedule.process.elapsed() >= PROCESS_POLL_INTERVAL
        {
            schedule.process = Instant::now();
            child
                .as_mut()
                .map(|child| child.try_wait())
                .transpose()
                .map_err(|error| format!("failed waiting for target process: {error}"))?
                .flatten()
        } else {
            None
        };
        if let Some(status) = child_status {
            let code = status.code().unwrap_or(1);
            child = None;
            if arguments.steam_app_id.is_none() {
                return Ok(code);
            }
            launcher_exit_code = Some(code);
            if let Some(watcher) = steam_watch.as_ref() {
                watcher.launcher_exited();
            }
        }
        if steam_watch
            .as_ref()
            .is_some_and(SteamWatcher::game_session_over)
        {
            return Ok(launcher_exit_code.unwrap_or(0));
        }
        // With the inotify pump waking the channel, saves apply instantly; the
        // periodic cadence is only the fallback when inotify is unavailable.
        let profile_wake_due = profile_wake_fd.is_some()
            || schedule.profile.elapsed() >= PROFILE_POLL_INTERVAL;
        if profile_monitor.is_some() && profile_wake_due {
            schedule.profile = Instant::now();
            if let Some(monitor) = profile_monitor.as_mut() {
                if monitor.changed() {
                    let reloaded = {
                        let mut outputs = LiveOutputs {
                            mapper: &mut mapper,
                            gamepad: &mut virtual_gamepad,
                            keyboard: &mut keyboard,
                            mouse: &mut mouse,
                            pad: &mut pad_state,
                            pipeline: &mut pipeline,
                            motion_enabled,
                            mirror: mirror.clone(),
                            native_passthrough,
                        };
                        reload_profile(&mut outputs, monitor.path(), &mut trace)
                    };
                    match reloaded {
                        Err(error) => eprintln!(
                            "ira-input: profile reload failed for {}: {error}",
                            monitor.path().display()
                        ),
                        Ok(()) => {
                            if let Some(events) = &arguments.events {
                                let _ = events.send(SessionEvent::ProfileReloaded {
                                    path: monitor.path().display().to_string(),
                                });
                            }
                            pipeline.gyro_processor = make_gyro_processor(
                                mapper.profile(),
                                pad_vendor,
                                pad_product,
                                arguments.calibration.as_deref(),
                            );
                            apply_controller_deadzone(
                                &mut mapper,
                                arguments.calibration.as_deref(),
                                pad_vendor,
                                pad_product,
                            );
                            pipeline.last_sensor_us = None;
                            tick_needed = tick_needed_for(&pipeline, &mapper);
                        }
                    }
                }
            }
        }
        // Paused ticks are skipped, so their deadline must not drive wakeups; the
        // focus entry keeps unpause latency at one poll interval.
        let timeout = floor_poll_timeout(schedule.timeout(LoopActivity {
            tick: tick_needed && !paused,
            tick_interval,
            child: child.is_some(),
            profile: profile_monitor.is_some() && profile_wake_fd.is_none(),
            cursor: cursor_watcher.is_some(),
            focus: focus.is_some(),
        }));
        match pad_events.recv_timeout(timeout.unwrap_or(IDLE_PARK)) {
            Ok(event) => pending.push(event),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
        }
    }
}

struct UnsubscribeOnDrop<'a>(&'a HubHandle, u64);

impl Drop for UnsubscribeOnDrop<'_> {
    fn drop(&mut self) {
        self.0.send(HubCommand::Unsubscribe(self.1));
    }
}

/// The overlay SHM name when the session runs under daemon-owned overlay control.
/// `None` otherwise.
pub(crate) fn daemon_toggle_shm_name(env: Option<&[(String, String)]>) -> Option<String> {
    let env = env?;
    env.iter()
        .any(|(key, value)| key == ira_overlay_ipc::DAEMON_TOGGLE_ENV && value == "1")
        .then(|| {
            env.iter()
                .find(|(key, _)| key == "IRA_OVERLAY_SHM")
                .map(|(_, name)| name.clone())
        })?
}fn release_outputs(
    mapper: &mut crate::MappingEngine,
    virtual_gamepad: &mut crate::VirtualGamepad,
    keyboard: &mut Option<crate::VirtualKeyboard>,
    mouse: &mut Option<crate::VirtualMouse>,
    pad_state: &mut crate::PadState,
    trace: &mut super::TraceState,
) -> Result<(), String> {
    emit_outputs(
        mapper.reset(),
        OutputTargets {
            gamepad: virtual_gamepad,
            keyboard: keyboard.as_mut(),
            mouse: mouse.as_mut(),
            pad: pad_state,
        },
        trace,
    )
}

#[cfg(test)]
mod tests {
    use super::daemon_toggle_shm_name;

    #[test]
    fn test_daemon_toggle_shm_name_needs_marker_and_mapping() {
        assert_eq!(daemon_toggle_shm_name(None), None);
        assert_eq!(daemon_toggle_shm_name(Some(&[])), None);
        let marked = vec![(ira_overlay_ipc::DAEMON_TOGGLE_ENV.to_string(), "1".to_string())];
        assert_eq!(daemon_toggle_shm_name(Some(&marked)), None);
        let armed = vec![
            (ira_overlay_ipc::DAEMON_TOGGLE_ENV.to_string(), "1".to_string()),
            ("IRA_OVERLAY_SHM".to_string(), "/ira_overlay_1".to_string()),
        ];
        assert_eq!(
            daemon_toggle_shm_name(Some(&armed)),
            Some("/ira_overlay_1".to_string())
        );
        let off = vec![
            (ira_overlay_ipc::DAEMON_TOGGLE_ENV.to_string(), "0".to_string()),
            ("IRA_OVERLAY_SHM".to_string(), "/ira_overlay_1".to_string()),
        ];
        assert_eq!(daemon_toggle_shm_name(Some(&off)), None);
    }
}
