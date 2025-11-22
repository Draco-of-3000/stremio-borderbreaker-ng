pub mod player;
pub use player::Player;
pub mod communication;
pub use communication::{
    CmdVal, PlayerEnded, PlayerEvent, PlayerProprChange, PlayerResponse, PropKey, PropVal,
};
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "c", content = "a")] // c=command, a=args
pub enum InMsg {
    MpvObserveProp(InMsgArgs),
    MpvSetProp(InMsgArgs),
    MpvCommand(InMsgArgs),
    WindowResized(InMsgArgs),
    CycleAspect(InMsgArgs),
    ToggleFill(InMsgArgs),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum InMsgArgs {
    ObProp(PropKey),
    StProp(String, PropVal),
    Cmd(CmdVal),
    WindowResized(u32, u32),
    None,
}
#[derive(Serialize, Deserialize, Debug)]
pub enum InMsgFn {
    MpvObserveProp,
    MpvSetProp,
    MpvCommand,
    WindowResized,
    CycleAspect,
    ToggleFill,
};
#[cfg(test)]
mod communication_tests;
