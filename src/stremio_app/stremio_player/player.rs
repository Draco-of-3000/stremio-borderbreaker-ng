use crate::stremio_app::ipc;
use crate::stremio_app::RPCResponse;
use flume::{Receiver, Sender};
use libmpv2::{events::Event, events::EventContext, Format, Mpv, SetData};
use native_windows_gui::{self as nwg, PartialUi};
use std::{
    sync::Arc,
    thread::{self, JoinHandle},
};
use winapi::shared::windef::HWND;

use crate::stremio_app::stremio_player::{
    CmdVal, InMsg, InMsgArgs, PlayerEnded, PlayerEvent, PlayerProprChange, PlayerResponse, PropKey,
    PropVal,
};
use serde_json::Value;

enum InternalEvent {
    VideoParamsChanged(Value),
}

struct BorderBreaker {
    aspect_mode: usize,
    modes: Vec<&'static str>,
    mode_ratios: Vec<f64>,
    // auto_detect: bool, // implied by aspect_mode == 0
    // show_overlay: bool, // assumed true
    current_video_params: Option<Value>,
    window_size: (u32, u32),
}

impl Default for BorderBreaker {
    fn default() -> Self {
        Self {
            aspect_mode: 0,
            modes: vec![
                "Auto",
                "Fill(Crop)",
                "Stretch",
                "16:9",
                "4:3",
                "1:1",
                "21:9 Ultrawide",
                "32:9 Super Ultrawide",
                "Cinema",
                "Original",
            ],
            mode_ratios: vec![
                0.0,
                -2.0,
                -3.0,
                16.0 / 9.0,
                4.0 / 3.0,
                1.0,
                21.0 / 9.0,
                32.0 / 9.0,
                2.39,
                -1.0,
            ],
            current_video_params: None,
            window_size: (0, 0),
        }
    }
}

impl BorderBreaker {
    fn apply_aspect(&mut self, mpv: &Mpv) {
        let mode = self.aspect_mode;
        let ratio = self.mode_ratios[mode];

        // Reset first
        let _ = mpv.set_property("panscan", 0.0);
        let _ = mpv.set_property("video-aspect-override", -1.0);

        if mode == 0 {
            // Auto
            if let Some(ref params) = self.current_video_params {
                self.check_auto_detect(params, mpv);
            }
        } else if ratio == -2.0 {
            // Fill (Crop)
            let _ = mpv.set_property("panscan", 1.0);
        } else if ratio == -3.0 {
            // Stretch
            if self.window_size.0 > 0 && self.window_size.1 > 0 {
                let screen_ratio = self.window_size.0 as f64 / self.window_size.1 as f64;
                let _ = mpv.set_property("video-aspect-override", screen_ratio);
            }
        } else if ratio > 0.0 {
            // Fixed Ratios
            let _ = mpv.set_property("video-aspect-override", ratio);
        }
    }

    fn check_auto_detect(&self, params: &Value, mpv: &Mpv) {
        if self.aspect_mode != 0 {
            return;
        }

        let dw = params.get("dw").and_then(|v| v.as_f64());
        let dh = params.get("dh").and_then(|v| v.as_f64());

        if let (Some(dw), Some(dh)) = (dw, dh) {
            if dw == 0.0 || dh == 0.0 || self.window_size.0 == 0 || self.window_size.1 == 0 {
                return;
            }

            let video_ratio = dw / dh;
            let screen_ratio = self.window_size.0 as f64 / self.window_size.1 as f64;
            let ratio_diff = (video_ratio - screen_ratio).abs();
            let tolerance = 0.05;

            // Reset first (already done in apply_aspect but good for safety if called independently)
            let _ = mpv.set_property("panscan", 0.0);
            let _ = mpv.set_property("video-aspect-override", -1.0);

            if ratio_diff < tolerance {
                return;
            }

            if video_ratio < screen_ratio {
                // Video is narrower than screen (pillarboxing) -> Crop to fill
                let _ = mpv.set_property("panscan", 1.0);
            }
            // If video is wider (letterboxing), we keep it (default behavior)
        }
    }

    fn cycle_aspect(&mut self, mpv: &Mpv) {
        self.aspect_mode = (self.aspect_mode + 1) % self.modes.len();
        self.apply_aspect(mpv);
        self.show_osd(mpv);
    }

    fn toggle_fill(&mut self, mpv: &Mpv) {
        self.aspect_mode = 1; // Fill
        self.apply_aspect(mpv);
        self.show_osd(mpv);
    }

    fn show_osd(&self, mpv: &Mpv) {
        let text = self.modes[self.aspect_mode];
        let _ = mpv.command("show-text", &[text]);
    }
}

struct ObserveProperty {
    name: String,
    format: Format,
}

#[derive(Default)]
pub struct Player {
    pub channel: ipc::Channel,
}

impl PartialUi for Player {
    fn build_partial<W: Into<nwg::ControlHandle>>(
        // @TODO replace with `&mut self`?
        data: &mut Self,
        parent: Option<W>,
    ) -> Result<(), nwg::NwgError> {
        // @TODO replace all `expect`s with proper error handling?

        let window_handle = parent
            .expect("no parent window")
            .into()
            .hwnd()
            .expect("cannot obtain window handle");

        let (in_msg_sender, in_msg_receiver) = flume::unbounded();
        let (rpc_response_sender, rpc_response_receiver) = flume::unbounded();
        let (observe_property_sender, observe_property_receiver) = flume::unbounded();
        let (internal_sender, internal_receiver) = flume::unbounded();
        data.channel = ipc::Channel::new(Some((in_msg_sender, rpc_response_receiver)));

        let mpv = create_shareable_mpv(window_handle);

        let _event_thread = create_event_thread(
            Arc::clone(&mpv),
            observe_property_receiver,
            rpc_response_sender,
            internal_sender,
        );
        let _message_thread = create_message_thread(
            mpv,
            observe_property_sender,
            in_msg_receiver,
            internal_receiver,
        );
        // @TODO implement a mechanism to stop threads on `Player` drop if needed

        Ok(())
    }
}

fn create_shareable_mpv(window_handle: HWND) -> Arc<Mpv> {
    let mpv = Mpv::with_initializer(|initializer| {
        macro_rules! set_property {
            ($name:literal, $value:expr) => {
                initializer
                    .set_property($name, $value)
                    .expect(concat!("failed to set ", $name));
            };
        }
        set_property!("wid", window_handle as i64);
        set_property!("title", "Stremio");
        set_property!("audio-client-name", "Stremio");
        set_property!("terminal", "yes");
        #[cfg(debug_assertions)]
        set_property!("msg-level", "all=no,cplayer=debug");
        #[cfg(not(debug_assertions))]
        set_property!("msg-level", "all=no");
        set_property!("quiet", "yes");
        set_property!("hwdec", "auto");
        // set_property!("vo", "gpu-next,");
        Ok(())
    });
    Arc::new(mpv.expect("cannot build MPV"))
}

fn create_event_thread(
    mpv: Arc<Mpv>,
    observe_property_receiver: Receiver<ObserveProperty>,
    rpc_response_sender: Sender<String>,
    internal_sender: Sender<InternalEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut event_context = EventContext::new(mpv.ctx);
        event_context
            .disable_deprecated_events()
            .expect("failed to disable deprecated MPV events");

        // -- Event handler loop --

        loop {
            for ObserveProperty { name, format } in observe_property_receiver.drain() {
                event_context
                    .observe_property(&name, format, 0)
                    .expect("failed to observer MPV property");
            }

            // -1.0 means to block and wait for an event.
            let event = match event_context.wait_event(-1.) {
                Some(Ok(event)) => event,
                Some(Err(error)) => {
                    eprintln!("Event errored: {error:?}");
                    continue;
                }
                // dummy event received (may be created on a wake up call or on timeout)
                None => continue,
            };

            // even if you don't do anything with the events, it is still necessary to empty the event loop
            let player_response = match event {
                Event::PropertyChange { name, change, .. } => {
                    let prop_change = PlayerProprChange::from_name_value(name.to_string(), change);
                    if name == "video-out-params" {
                        internal_sender
                            .send(InternalEvent::VideoParamsChanged(
                                prop_change.data().clone(),
                            ))
                            .ok();
                    }

                    PlayerResponse("mpv-prop-change", PlayerEvent::PropChange(prop_change))
                }
                Event::EndFile(reason) => PlayerResponse(
                    "mpv-event-ended",
                    PlayerEvent::End(PlayerEnded::from_end_reason(reason)),
                ),
                Event::Shutdown => {
                    break;
                }
                _ => continue,
            };

            rpc_response_sender
                .send(RPCResponse::response_message(player_response.to_value()))
                .expect("failed to send RPCResponse");
        }
    })
}

fn create_message_thread(
    mpv: Arc<Mpv>,
    observe_property_sender: Sender<ObserveProperty>,
    in_msg_receiver: Receiver<String>,
    internal_receiver: Receiver<InternalEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut bb = BorderBreaker::default();
        // -- Helpers --

        let observe_property = |name: String, format: Format| {
            observe_property_sender
                .send(ObserveProperty { name, format })
                .expect("cannot send ObserveProperty");
            mpv.wake_up();
        };

        // Observe video-out-params for BorderBreaker
        observe_property("video-out-params".to_string(), Format::Node);

        let send_command = |cmd: CmdVal| {
            let a1;
            let a2;
            let a3;
            let a4;
            let (name, args) = match cmd {
                CmdVal::Quintuple(name, arg1, arg2, arg3, arg4) => {
                    a1 = format!(r#""{arg1}""#);
                    a2 = format!(r#""{arg2}""#);
                    a3 = format!(r#""{arg3}""#);
                    a4 = format!(r#""{arg4}""#);
                    (
                        name,
                        vec![a1.as_ref(), a2.as_ref(), a3.as_ref(), a4.as_ref()],
                    )
                }
                CmdVal::Quadruple(name, arg1, arg2, arg3) => {
                    a1 = format!(r#""{arg1}""#);
                    a2 = format!(r#""{arg2}""#);
                    a3 = format!(r#""{arg3}""#);
                    (name, vec![a1.as_ref(), a2.as_ref(), a3.as_ref()])
                }
                CmdVal::Tripple(name, arg1, arg2) => {
                    a1 = format!(r#""{arg1}""#);
                    a2 = format!(r#""{arg2}""#);
                    (name, vec![a1.as_ref(), a2.as_ref()])
                }
                CmdVal::Double(name, arg1) => {
                    a1 = format!(r#""{arg1}""#);
                    (name, vec![a1.as_ref()])
                }
                CmdVal::Single((name,)) => (name, vec![]),
            };
            if let Err(error) = mpv.command(&name.to_string(), &args) {
                eprintln!("failed to execute MPV command: '{error:#}'")
            }
        };

        fn set_property(name: impl ToString, value: impl SetData, mpv: &Mpv) {
            if let Err(error) = mpv.set_property(&name.to_string(), value) {
                eprintln!("cannot set MPV property: '{error:#}'")
            }
        }

        // -- InMsg handler loop --

        enum MessageType {
            InMsg(String),
            Internal(InternalEvent),
            Exit,
        }

        loop {
            let event = flume::Selector::new()
                .recv(&in_msg_receiver, |msg| match msg {
                    Ok(msg) => MessageType::InMsg(msg),
                    Err(_) => MessageType::Exit,
                })
                .recv(&internal_receiver, |msg| match msg {
                    Ok(msg) => MessageType::Internal(msg),
                    Err(_) => MessageType::Exit,
                })
                .wait();

            match event {
                MessageType::InMsg(msg) => {
                    let in_msg: InMsg = match serde_json::from_str(&msg) {
                        Ok(in_msg) => in_msg,
                        Err(error) => {
                            eprintln!("cannot parse InMsg:{:?} {error:#}", &msg);
                            continue;
                        }
                    };

                    match in_msg {
                        InMsg::MpvObserveProp(InMsgArgs::ObProp(PropKey::Bool(prop))) => {
                            observe_property(prop.to_string(), Format::Flag);
                        }
                        InMsg::MpvObserveProp(InMsgArgs::ObProp(PropKey::Int(prop))) => {
                            observe_property(prop.to_string(), Format::Int64);
                        }
                        InMsg::MpvObserveProp(InMsgArgs::ObProp(PropKey::Fp(prop))) => {
                            observe_property(prop.to_string(), Format::Double);
                        }
                        InMsg::MpvObserveProp(InMsgArgs::ObProp(PropKey::Str(prop))) => {
                            observe_property(prop.to_string(), Format::String);
                        }
                        InMsg::MpvSetProp(InMsgArgs::StProp(name, PropVal::Bool(value))) => {
                            set_property(name, value, &mpv);
                        }
                        InMsg::MpvSetProp(InMsgArgs::StProp(name, PropVal::Num(value))) => {
                            set_property(name, value, &mpv);
                        }
                        InMsg::MpvSetProp(InMsgArgs::StProp(name, PropVal::Str(value))) => {
                            let value = if name.to_string() == "vo" {
                                let mut value = value;
                                if !value.is_empty() && !value.ends_with(',') {
                                    value.push(',');
                                }
                                value.push_str("gpu-next,");
                                value
                            } else {
                                value
                            };
                            set_property(name, value, &mpv);
                        }
                        InMsg::MpvCommand(InMsgArgs::Cmd(cmd)) => {
                            send_command(cmd);
                        }
                        InMsg::WindowResized(InMsgArgs::WindowResized(w, h)) => {
                            bb.window_size = (w, h);
                            bb.apply_aspect(&mpv);
                        }
                        InMsg::CycleAspect(_) => {
                            bb.cycle_aspect(&mpv);
                        }
                        InMsg::ToggleFill(_) => {
                            bb.toggle_fill(&mpv);
                        }
                        msg => {
                            eprintln!("MPV unsupported message: '{msg:?}'");
                        }
                    }
                }
                MessageType::Internal(event) => match event {
                    InternalEvent::VideoParamsChanged(params) => {
                        bb.current_video_params = Some(params.clone());
                        bb.apply_aspect(&mpv);
                    }
                },
                MessageType::Exit => break,
            }
        }
    })
}

trait MpvExt {
    fn wake_up(&self);
}

impl MpvExt for Mpv {
    // @TODO create a PR to the `libmpv` crate and then remove `libmpv-sys` from Cargo.toml?
    fn wake_up(&self) {
        unsafe { libmpv2_sys::mpv_wakeup(self.ctx.as_ptr()) }
    }
}
