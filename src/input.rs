
use gtk::prelude::*;
use gdk;
use gdk::EventKey;
use phf;
use crate::nvim::{NvimSession, ErrorReport};

include!(concat!(env!("OUT_DIR"), "/key_map_table.rs"));

pub fn keyval_to_input_string(in_str: &str, in_state: gdk::ModifierType) -> String {
    let mut val = in_str;
    let mut state = in_state;
    let empty = in_str.is_empty();

    if !empty {
        debug!("keyval -> {}", in_str);
    }

    // CTRL-^ and CTRL-@ don't work in the normal way.
    if state.contains(gdk::ModifierType::CONTROL_MASK) && !state.contains(gdk::ModifierType::SHIFT_MASK) &&
        !state.contains(gdk::ModifierType::MOD1_MASK)
    {
        if val == "6" {
            val = "^";
        } else if val == "2" {
            val = "@";
        }
    }

    let chars: Vec<char> = in_str.chars().collect();

    if chars.len() == 1 {
        let ch = chars[0];

        // Remove SHIFT
        if ch.is_ascii() && !ch.is_alphanumeric() {
            state.remove(gdk::ModifierType::SHIFT_MASK);
        }
    }

    if val == "<" {
        val = "lt";
    }

    let mut mod_chars = Vec::<&str>::with_capacity(3);
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        mod_chars.push("S");
    }
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        mod_chars.push("C");
    }
    if state.contains(gdk::ModifierType::MOD1_MASK) {
        mod_chars.push("A");
    }

    let sep = if empty { "" } else { "-" };
    let input = [mod_chars.as_slice(), &[val]].concat().join(sep);

    if !empty && input.chars().count() > 1 {
        format!("<{}>", input)
    } else {
        input
    }
}

pub fn convert_key(ev: &EventKey) -> Option<String> {
    let keyval = ev.get_keyval();
    let state = ev.get_state();
    if let Some(ref keyval_name) = gdk::keyval_name(keyval) {
        if let Some(cnvt) = KEYVAL_MAP.get(keyval_name as &str).cloned() {
            return Some(keyval_to_input_string(cnvt, state));
        }
    }

    if let Some(ch) = gdk::keyval_to_unicode(keyval) {
        Some(keyval_to_input_string(&ch.to_string(), state))
    } else {
        None
    }
}

pub fn im_input(nvim: &NvimSession, input: &str) {
    debug!("nvim_input -> {}", input);

    let input: String = input
        .chars()
        .map(|ch| {
            keyval_to_input_string(&ch.to_string(), gdk::ModifierType::empty())
        })
        .collect();
    nvim
        .block_timeout(nvim.input(&input))
        .ok_and_report()
        .expect("Failed to send input command to nvim");
}

pub fn gtk_key_press(nvim: &NvimSession, ev: &EventKey)
    -> Inhibit {
    if let Some(input) = convert_key(ev) {
        debug!("nvim_input -> {}", input);
        nvim
            .block_timeout(nvim.input(&input))
            .ok_and_report()
            .expect("Failed to send input command to nvim");
        Inhibit(true)
    } else {
        Inhibit(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyval_to_input_string() {
        macro_rules! test {
            ( $( $in_str:literal $( , $( $mod:ident )|* )? == $out_str:literal );*; ) => {
                let mut modifier;
                $(
                    modifier = gdk::ModifierType::empty() $( | $( gdk::ModifierType::$mod )|* )?;
                    assert_eq!(keyval_to_input_string($in_str, modifier), $out_str)
                );*
            }
        }

        test! {
            "a" == "a";
            "" == "";
            "6" == "6";
            "2" == "2";
            "<" == "<lt>";
            "", SHIFT_MASK == "S";
            "", SHIFT_MASK | CONTROL_MASK | MOD1_MASK == "SCA";
            "a", SHIFT_MASK == "<S-a>";
            "a", SHIFT_MASK | CONTROL_MASK | MOD1_MASK == "<S-C-A-a>";
            "6", CONTROL_MASK == "<C-^>";
            "6", CONTROL_MASK | MOD1_MASK == "<C-A-6>";
            "2", CONTROL_MASK == "<C-@>";
            "2", CONTROL_MASK | MOD1_MASK == "<C-A-2>";
        }
    }
}
