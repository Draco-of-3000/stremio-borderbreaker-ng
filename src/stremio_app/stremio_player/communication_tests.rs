use crate::stremio_app::stremio_player::{
    communication::{BoolProp, CmdVal, MpvCmd, PlayerEnded, PlayerProprChange, PropKey, PropVal},
    InMsg, InMsgArgs,
};
use libmpv2::{events::PropertyData, mpv_end_file_reason};

use serde_json::json;
use serde_test::{assert_tokens, Token};

#[test]
fn propr_change_tokens() {
    let prop = "test-prop";
    let tokens: [Token; 6] = [
        Token::Struct {
            name: "PlayerProprChange",
            len: 2,
        },
        Token::Str("name"),
        Token::None,
        Token::Str("data"),
        Token::None,
        Token::StructEnd,
    ];

    fn tokens_by_type(tokens: &[Token; 6], name: &'static str, val: PropertyData, token: Token) {
        let mut typed_tokens = tokens.clone();
        typed_tokens[2] = Token::Str(name);
        typed_tokens[4] = token;
        assert_tokens(
            &PlayerProprChange::from_name_value(name.to_string(), val),
            &typed_tokens,
        );
    }
    tokens_by_type(&tokens, prop, PropertyData::Flag(true), Token::Bool(true));
    tokens_by_type(&tokens, prop, PropertyData::Int64(1), Token::F64(1.0));
    tokens_by_type(&tokens, prop, PropertyData::Double(1.0), Token::F64(1.0));
    tokens_by_type(&tokens, prop, PropertyData::OsdStr("ok"), Token::Str("ok"));
    tokens_by_type(&tokens, prop, PropertyData::Str("ok"), Token::Str("ok"));

    // JSON response
    tokens_by_type(
        &tokens,
        "track-list",
        PropertyData::Str(r#""ok""#),
        Token::Str("ok"),
    );
    tokens_by_type(
        &tokens,
        "video-params",
        PropertyData::Str(r#""ok""#),
        Token::Str("ok"),
    );
    tokens_by_type(
        &tokens,
        "metadata",
        PropertyData::Str(r#""ok""#),
        Token::Str("ok"),
    );
}

#[test]
fn ended_tokens() {
    let tokens: [Token; 4] = [
        Token::Struct {
            name: "PlayerEnded",
            len: 1,
        },
        Token::Str("reason"),
        Token::None,
        Token::StructEnd,
    ];
    let mut typed_tokens = tokens.clone();
    typed_tokens[2] = Token::Str("error");
    assert_tokens(
        &PlayerEnded::from_end_reason(mpv_end_file_reason::Error),
        &typed_tokens,
    );
    let mut typed_tokens = tokens.clone();
    typed_tokens[2] = Token::Str("quit");
    assert_tokens(
        &PlayerEnded::from_end_reason(mpv_end_file_reason::Quit),
        &typed_tokens,
    );
}

#[test]
fn ob_prop_serialization() {
    let msg = InMsg::MpvObserveProp(InMsgArgs::ObProp(PropKey::Bool(BoolProp::Pause)));
    let value = serde_json::to_value(&msg).unwrap();
    assert_eq!(value, json!({"c":"MpvObserveProp","a":"pause"}));
}

#[test]
fn set_prop_serialization() {
    let msg = InMsg::MpvSetProp(InMsgArgs::StProp("pause".to_string(), PropVal::Bool(true)));
    let value = serde_json::to_value(&msg).unwrap();
    assert_eq!(value, json!({"c":"MpvSetProp","a":["pause",true]}));
}

#[test]
fn command_stop_serialization() {
    let msg = InMsg::MpvCommand(InMsgArgs::Cmd(CmdVal::Single((MpvCmd::Stop,))));
    let value = serde_json::to_value(&msg).unwrap();
    assert_eq!(value, json!({"c":"MpvCommand","a":["stop"]}));
}

#[test]
fn command_loadfile_serialization() {
    let msg = InMsg::MpvCommand(InMsgArgs::Cmd(CmdVal::Double(
        MpvCmd::Loadfile,
        "some_file".to_string(),
    )));
    let value = serde_json::to_value(&msg).unwrap();
    assert_eq!(
        value,
        json!({"c":"MpvCommand","a":["loadfile","some_file"]})
    );
}
