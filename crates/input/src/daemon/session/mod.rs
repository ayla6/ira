mod devices;
mod inputs;
mod output_stack;

// The hub owns the physical side now; these device helpers are its tools.
pub(crate) use devices::{
    apply_controller_layout, open_rumble, open_sensor, reconnect_gamepad, resolved_layout_for,
    GyroSource,
};
pub(crate) use sensor::now_us;
mod loop_io;
mod outputs;
mod sensor;
mod setup;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::hub::{HubCommand, HubHandle, PadEvent};
use super::{Arguments, SessionEvent, SteamWatcher};
use super::signals::{install_signal_handlers, STOP_REQUESTED};
use devices::{apply_controller_deadzone, make_gyro_processor};
use inputs::{poll_cursor_switch, process_pad_events};
use loop_io::{
    floor_poll_timeout, FOCUS_POLL_INTERVAL, LoopActivity, PROCESS_POLL_INTERVAL,
    PROFILE_POLL_INTERVAL,
};
use outputs::{emit_outputs, reload_profile, stop_child, LiveOutputs, OutputTargets};
use sensor::{process_tick, service_twin_events, tick_needed_for};
use setup::{setup_session, SessionSetup};

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
        mut keyboard,
        mut mouse,
        mut virtual_gamepad,
        mut pad_state,
        mut pipeline,
        motion_enabled,
        mut pad_vendor,
        mut pad_product,
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
    // Motion samples queued since the last tick. Every sample must be
    // processed: dropping one stretches the next integration window over
    // its time and turns real micro-motion into phantom rotation.
    let mut samples: Vec<crate::SensorSample> = Vec::new();

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
            // The hub only routes to sessions it believes are focused, so
            // every change from the last reported state must be reported —
            // including the first reading, which claims the pad for a game
            // that launched focused.
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
        // Keep every uhid twin's kernel conversation moving regardless of
        // pause or tick gating: hid-nintendo's connect handshake fails the
        // driver probe within seconds, SDL feature probes arrive whenever a
        // reader opens a pad, and queued output events otherwise stall the
        // rumble replay path.
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
                    tick_needed = tick_needed_for(&pipeline, &mapper);
                }
                PadEvent::Connected {
                    name,
                    path,
                    vendor,
                    product,
                } => {
                    pad_vendor = vendor;
                    pad_product = product;
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
                    pipeline.last_sensor_us = None;
                    report_rate.reset();
                    if let Some(events) = &arguments.events {
                        let _ = events.send(SessionEvent::Controller {
                            connected: true,
                            name,
                            path,
                        });
                    }
                }
                PadEvent::ProfileChanged => {}
                PadEvent::Frozen => {
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
                PadEvent::Live => hub_live = true,
            }
        }

        let paused = paused_for_focus || !hub_live;
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
                    std::mem::take(&mut samples),
                )
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
        // With the inotify pump waking the channel, profile saves apply
        // instantly; the periodic cadence only remains as the fallback when
        // inotify is unavailable.
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
        // While paused the ticks are skipped, so their deadline must not
        // drive the wakeup rate; the focus entry keeps unpause latency at
        // one poll interval.
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

fn release_outputs(
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
