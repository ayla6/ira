mod devices;
mod inputs;
mod loop_io;
mod outputs;
mod sensor;
mod setup;

use std::sync::atomic::Ordering;
use std::time::Instant;

use super::{Arguments, SessionEvent, SteamWatcher};
use super::signals::{install_signal_handlers, STOP_REQUESTED};
use devices::{
    apply_controller_deadzone, apply_controller_layout, make_gyro_processor,
    open_rumble, open_sensor, play_physical_rumble, reconnect_gamepad, resolved_layout_for,
};
use inputs::{poll_cursor_switch, process_physical_inputs};
use loop_io::{
    floor_poll_timeout, wait_for_inputs, FOCUS_POLL_INTERVAL, LoopActivity,
    PROCESS_POLL_INTERVAL, PROFILE_POLL_INTERVAL, RECONNECT_INTERVAL,
    SWITCH_DRIVER_SILENCE_LIMIT,
};
use outputs::{emit_outputs, reload_profile, stop_child, OutputTargets};
use sensor::{open_motion_node, process_tick, service_twin_events};
use setup::{setup_session, SessionSetup};

pub fn run_session(arguments: Arguments) -> Result<i32, String> {
    STOP_REQUESTED.store(false, Ordering::Relaxed);
    install_signal_handlers();
    let SessionSetup {
        mut trace,
        mut gamepad,
        mut mapper,
        mut profile_monitor,
        mut keyboard,
        mut mouse,
        mut virtual_gamepad,
        mut pad_state,
        mut pipeline,
        mut rumble_output,
        motion_enabled,
        pad_vendor,
        pad_product,
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
    // Parked in the session loop's poll set so profile saves apply instantly.
    let profile_wake_fd: Option<libc::c_int> =
        profile_monitor.as_ref().and_then(|monitor| monitor.fd());

    loop {
        let was_connected = gamepad
            .as_ref()
            .is_some_and(|gamepad| gamepad.is_connected());
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
            if !focused && !paused_for_focus {
                paused_for_focus = true;
                eprintln!("ira-input: game window unfocused; pausing input");
                if let Err(error) = emit_outputs(
                    mapper.reset(),
                    OutputTargets {
                        gamepad: &mut virtual_gamepad,
                        keyboard: keyboard.as_mut(),
                        mouse: mouse.as_mut(),
                        pad: &mut pad_state,
                    },
                    &mut trace,
                ) {
                    eprintln!("ira-input: failed to release outputs on pause: {error}");
                }
                if let Some(rumble) = rumble_output.as_mut() {
                    rumble.stop();
                }
                if let Some(switch) = pipeline.switch_hidraw.as_mut() {
                    switch.stop_rumble();
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
        // Fail-safe for the Switch hidraw takeover: a pad that streamed
        // once and then went silent would otherwise starve the session of
        // input entirely, since the takeover replaced the evdev path.
        // Drop the driver, fall back to evdev, and let rumble take the
        // ordinary path again.
        if pipeline
            .switch_hidraw
            .as_ref()
            .is_some_and(|driver| driver.silent_for() >= SWITCH_DRIVER_SILENCE_LIMIT)
        {
            eprintln!(
                "ira-input: Switch hidraw driver went silent; \
                 falling back to the evdev input path"
            );
            pipeline.switch_hidraw = None;
            rumble_output = open_rumble(gamepad.as_ref(), mapper.profile().rumble);
        }
        // Vendor hidraw rumble has no kernel timer: check the deadline every
        // pass so a game that stops replaying its effect cannot leave the
        // motors running.
        if let Some(rumble) = rumble_output.as_mut() {
            rumble.service();
        }
        let result = if paused_for_focus {
            // The pad keeps streaming while its events are ignored. If those
            // reports stayed queued, the fd would read POLLIN forever and
            // poll would return instantly on every pass — the loop would
            // free-run at full speed doing nothing (one core at 100%). Drain
            // and discard instead; mapping state stays in sync for the
            // unpause.
            if let Some(gamepad) = gamepad.as_mut() {
                if let Err(error) = gamepad.fetch_events() {
                    eprintln!("ira-input: failed draining paused controller: {error}");
                }
            }
            if let Some(switch) = pipeline.switch_hidraw.as_mut() {
                let _ = switch.take_events();
            }
            Ok(())
        } else {
            process_physical_inputs(
                &mut gamepad,
                &mut pipeline,
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
                )
            })
        };
        trace.flush();
        // Rumble uploads are drained every pass so the kernel queue never
        // overflows, even while paused — commands just land nowhere then.
        for command in virtual_gamepad.poll_rumble() {
            if !paused_for_focus {
                play_physical_rumble(&mut pipeline, &mut rumble_output, command);
            }
        }
        // Native transports carry rumble inside HID output reports instead
        // (the uinput pad does not exist then); drain them the same way.
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
            if !paused_for_focus {
                play_physical_rumble(&mut pipeline, &mut rumble_output, command);
            }
        }
        if let Err(error) = result {
            let resets = emit_outputs(
                mapper.reset(),
                OutputTargets {
                    gamepad: &mut virtual_gamepad,
                    keyboard: keyboard.as_mut(),
                    mouse: mouse.as_mut(),
                    pad: &mut pad_state,
                },
                &mut trace,
            );
            if let Err(reset_error) = resets {
                eprintln!("ira-input: failed to emit reset releases: {reset_error}");
            }
            stop_child(&mut child);
            return Err(error);
        }
        let connected = gamepad
            .as_ref()
            .is_some_and(|gamepad| gamepad.is_connected());
        if was_connected && !connected {
            if let Some(events) = &arguments.events {
                let _ = events.send(SessionEvent::Controller {
                    connected: false,
                    name: gamepad
                        .as_ref()
                        .map(|gamepad| gamepad.info().name.clone())
                        .unwrap_or_default(),
                    path: String::new(),
                });
            }
            pipeline.sensor = None;
            pipeline.last_sensor_us = None;
            // A lost sensor must not stop frame servicing: gyroless DSU
            // sessions deliberately keep streaming whole-controller frames.
            tick_needed = pipeline.ever_had_sensor || mapper.has_continuous_outputs();
            schedule.reconnect = Instant::now();
            if let Some(rumble) = rumble_output.as_mut() {
                rumble.stop();
            }
            if let Some(switch) = pipeline.switch_hidraw.as_mut() {
                switch.stop_rumble();
            }
            emit_outputs(
                mapper.reset(),
                OutputTargets {
                    gamepad: &mut virtual_gamepad,
                    keyboard: keyboard.as_mut(),
                    mouse: mouse.as_mut(),
                    pad: &mut pad_state,
                },
                &mut trace,
            )?;
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
        if !connected && schedule.reconnect.elapsed() >= RECONNECT_INTERVAL {
            schedule.reconnect = Instant::now();
            pipeline.sensor = None;
            pipeline.switch_hidraw = None;
            match reconnect_gamepad(&mut gamepad) {
                Ok(true) => {
                    apply_controller_layout(&mut gamepad, arguments.calibration.as_deref());
                    if let Some(gamepad) = gamepad.as_ref() {
                        eprintln!(
                            "ira-input: controller connected through {}",
                            gamepad.info().path.display()
                        );
                        pipeline.sensor = open_sensor(gamepad.info());
                        if pipeline.sensor.is_none() {
                            pipeline.switch_hidraw =
                                crate::SwitchHidrawPad::open(gamepad.info());
                            if let Some(driver) = pipeline.switch_hidraw.as_mut() {
                                driver.set_nintendo_layout(resolved_layout_for(
                                    gamepad.info(),
                                    arguments.calibration.as_deref(),
                                ));
                            }
                        }
                        rumble_output = if pipeline.switch_hidraw.is_some() {
                            None
                        } else {
                            open_rumble(Some(gamepad), mapper.profile().rumble)
                        };
                        if pipeline.motion_device.is_none() && mapper.profile().native_motion {
                            // Best effort: if the game already opened the pad
                            // before this node appears, pairing waits for the
                            // next pad (re)open.
                            pipeline.motion_device = open_motion_node(mapper.profile().backend);
                        }
                        if pipeline.motion_alive()
                            && pipeline.motion.is_none()
                            && motion_enabled
                        {
                            pipeline.motion = crate::MotionServer::bind();
                            if pipeline.motion.is_some() {
                                eprintln!(
                                    "ira-input: motion passthrough on udp/{} for emulators (cemuhook)",
                                    crate::MOTION_PORT
                                );
                            }
                        }
                        pipeline.last_sensor_us = None;
                        tick_needed = pipeline.motion_alive()
                            || pipeline.motion.is_some()
                            || mapper.has_continuous_outputs();
                        report_rate.reset();
                    }
                    if let Some(events) = &arguments.events {
                        let _ = events.send(SessionEvent::Controller {
                            connected: true,
                            name: gamepad
                                .as_ref()
                                .map(|gamepad| gamepad.info().name.clone())
                                .unwrap_or_default(),
                            path: gamepad
                                .as_ref()
                                .map(|gamepad| gamepad.info().path.display().to_string())
                                .unwrap_or_default(),
                        });
                    }
                    schedule.sensor = Instant::now();
                    if let Some(gamepad) = gamepad.as_mut() {
                        if let Err(error) = gamepad.grab() {
                            eprintln!("ira-input: failed to grab controller: {error}");
                        }
                    }
                }
                Ok(false) => {}
                Err(error) => eprintln!("ira-input: controller reconnect failed: {error}"),
            }
        }
        // With the inotify descriptor parked in the poll set, writes wake
        // the loop directly and every pass can drain; the periodic cadence
        // only remains as the fallback when inotify is unavailable.
        let profile_wake_due = profile_wake_fd.is_some()
            || schedule.profile.elapsed() >= PROFILE_POLL_INTERVAL;
        if profile_monitor.is_some() && profile_wake_due {
            schedule.profile = Instant::now();
            if let Some(monitor) = profile_monitor.as_mut() {
                if monitor.changed() {
                    if let Err(error) = reload_profile(
                        &mut mapper,
                        &mut virtual_gamepad,
                        &mut keyboard,
                        &mut mouse,
                        &mut pad_state,
                        monitor.path(),
                        &mut trace,
                    ) {
                        eprintln!(
                            "ira-input: profile reload failed for {}: {error}",
                            monitor.path().display()
                        );
                    } else {
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
                        tick_needed = pipeline.sensor.is_some()
                            || pipeline.motion.is_some()
                            || mapper.has_continuous_outputs();
                        rumble_output = open_rumble(gamepad.as_ref(), mapper.profile().rumble);
                    }
                }
            }
        }
        // While paused the ticks are skipped, so their deadline must not
        // drive the wakeup rate; the focus entry keeps unpause latency at
        // one poll interval.
        let timeout = floor_poll_timeout(schedule.timeout(LoopActivity {
            tick: tick_needed && !paused_for_focus,
            tick_interval,
            child: child.is_some(),
            profile: profile_monitor.is_some() && profile_wake_fd.is_none(),
            disconnected: !connected,
            cursor: cursor_watcher.is_some(),
            focus: focus.is_some(),
        }));
        wait_for_inputs(
            &gamepad,
            profile_wake_fd.as_slice(),
            timeout,
        )?;
    }
}
