//! The gate as the domain sees it: the mixer's gate (`remux_mixer::gate`),
//! and the boundary where a socket message becomes a change to it.

pub use remux_mixer::gate::*;

/// The boundary with whatever is driving us: a socket message is untrusted, so
/// only the numeric gate keys survive it.
pub fn parse_gate_params(raw: &serde_json::Value) -> GatePatch {
    let mut patch = GatePatch::default();
    let Some(object) = raw.as_object() else {
        return patch;
    };
    let take = |key: &str| object.get(key).and_then(serde_json::Value::as_f64);
    // Two spellings: the status's own, which the panel and the CLI send back,
    // and the studio page's camelCase, which came first. Only the first
    // was read for a while, and a slider for a field with an underscore in
    // its name was accepted, ignored, and back where it was a second later.
    let either = |snake: &str, camel: &str| take(snake).or_else(|| take(camel));
    patch.hf = take("hf");
    patch.full = take("full");
    patch.floor = take("floor");
    patch.hold_ms = either("hold_ms", "holdMs");
    patch.attack_ms = either("attack_ms", "attackMs");
    patch.hf_attack_ms = either("hf_attack_ms", "hfAttackMs");
    patch.keys_boost = either("keys_boost", "keysBoost");
    patch
}

#[cfg(test)]
mod tests {
    use super::*;

    // The wire spells the fields the way the status does, attack_ms and
    // keys_boost; the parser only knew the studio page's attackMs and
    // keysBoost, so a panel's Attack and Keys boost sliders were accepted and
    // ignored, and snapped back a second later. Both spellings are taken.
    #[test]
    fn a_patch_is_read_in_the_status_s_own_spelling_as_well_as_the_page_s() {
        let snake = parse_gate_params(&serde_json::json!({
            "attack_ms": 80.0, "keys_boost": 3.0, "hold_ms": 300.0, "hf_attack_ms": 20.0
        }));
        assert_eq!(snake.attack_ms, Some(80.0));
        assert_eq!(snake.keys_boost, Some(3.0));
        assert_eq!(snake.hold_ms, Some(300.0));
        assert_eq!(snake.hf_attack_ms, Some(20.0));
        let camel = parse_gate_params(&serde_json::json!({ "attackMs": 80.0, "keysBoost": 3.0 }));
        assert_eq!(camel.attack_ms, Some(80.0));
        assert_eq!(camel.keys_boost, Some(3.0));
    }

    #[test]
    fn parse_gate_params_keeps_only_the_numeric_keys_from_an_untrusted_message() {
        let patch = parse_gate_params(&serde_json::json!({
            "hf": 0.5, "full": "loud", "bogus": 1, "holdMs": 300
        }));
        assert_eq!(
            patch,
            GatePatch {
                hf: Some(0.5),
                hold_ms: Some(300.0),
                ..Default::default()
            }
        );
        assert_eq!(
            parse_gate_params(&serde_json::Value::Null),
            GatePatch::default()
        );
        assert_eq!(
            parse_gate_params(&serde_json::json!("nope")),
            GatePatch::default()
        );
    }
}
