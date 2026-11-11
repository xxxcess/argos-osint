//! Fleetworks brand mark for interactive Venture sessions.

use std::env;

const GOLD: &str = "\x1b[38;5;179m";
const IVORY: &str = "\x1b[38;5;230m";
const RESET: &str = "\x1b[0m";

pub(crate) const MARK: &str = r#"
                     /\
                    /  \
        |\         /    \          /|
        | \_      /______\       _/ |
         \   \                 /   /
          \ __/               \__ /
           \/     #########     \/
                 #@MMMMMMM@#
 _______/    #@MMMMMMMMMMMMMMM@#    \_______
  \     /  #@MMMMMMMMMMMMMMMMMMM@#  \     /
   \   /  #@MMMMMMMMMMMMMMMMMMMMM@#  \   /
    \ /  @MMMMMMMMMMMMMMMMMMMMMMMMM@  \ /
     \/  @MMMMMMMMMMMMMMMMMMMMMMMMM@  \/
   *     @MMMMMMMMMMMMMMMMMMMMMMMMM@     *
   **     #@MMMMMMMMMMMMMMMMMMMMM@#     **
   *+*     #@MMMMMMMMMMMMMMMMMMM@#     *+*
   *+%+     #@@MMMMMMMMMMMMMMM@@#     +%+*
    *+%+:    ##@@@@@@@@@@@@@@@##    :+%+*
    **+%+:.    ###############    .:+%+**
     *++%+%:.     #########     .:%+%++*
      **+%%+%*+:.           .:+*%+%%+**
       **+%+%%+%*=**:*:=*:.::+*%+%+**
        **+%%++%*+=::..::=+*%++%%+**
          **+%%++%*=::..:=*%++%%+**
            ***+**+:....:+**+***
               ***+*:::::*+***

            F L E E T W O R K S
"#;

pub(crate) fn print() {
    if env::var_os("NO_COLOR").is_none()
        && env::var_os("TERM").is_some_and(|term| !term.is_empty() && term != "dumb")
    {
        eprint!("{}", colored_mark());
    } else {
        eprint!("{MARK}");
    }
}

fn colored_mark() -> String {
    let mut output = String::new();
    let mut current_color = "";
    for glyph in MARK.chars() {
        if !glyph.is_ascii_whitespace() {
            let color = if glyph.is_ascii_alphabetic() || matches!(glyph, '#' | '@') {
                IVORY
            } else {
                GOLD
            };
            if color != current_color {
                output.push_str(color);
                current_color = color;
            }
        }
        output.push(glyph);
    }
    output.push_str(RESET);
    output
}

#[cfg(test)]
mod tests {
    use super::{colored_mark, GOLD, IVORY, MARK, RESET};

    #[test]
    fn brand_splash_uses_ascii_and_preserves_art_in_color() {
        assert!(MARK.is_ascii());
        assert!(MARK.contains("F L E E T W O R K S"));
        let color = colored_mark();
        assert!(color.contains(GOLD) && color.contains(IVORY) && color.ends_with(RESET));
        assert_eq!(
            color
                .replace(GOLD, "")
                .replace(IVORY, "")
                .replace(RESET, ""),
            MARK
        );
    }
}
