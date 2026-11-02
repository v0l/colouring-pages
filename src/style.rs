pub const STYLE_SUFFIX: &str = "black and white line art colouring page for children, thick clean outlines, no shading, no grey, no filled black areas, pure white background, large closed regions that are easy to colour in, cute, simple, no text, no letters";

pub const NEGATIVE: &str = "shading, grey, gray, gradient, colour, color, filled black areas, texture, hatching, crosshatch, tiny details, text, letters, numbers, watermark, signature, photo, realistic";

const SCENE_MAX: usize = 700;

pub fn finish(scene: &str) -> String {
    let scene = scene.trim();
    let scene = scene.strip_suffix(STYLE_SUFFIX).unwrap_or(scene);
    let scene = scene.trim().trim_end_matches(['.', ',', ';', ':', ' ']);
    let scene: String = scene.chars().take(SCENE_MAX).collect();
    let scene = scene.trim_end_matches(['.', ',', ';', ':', ' ']);
    if scene.is_empty() {
        STYLE_SUFFIX.to_string()
    } else {
        format!("{scene}. {STYLE_SUFFIX}")
    }
}

pub fn with_negative_inline(prompt: &str) -> String {
    format!("{prompt}. Do not include: {NEGATIVE}.")
}

const BLOCKED: &[&str] = &[
    "nude", "naked", "nsfw", "porn", "sex", "sexy", "erotic", "gore", "gory", "bloody", "blood",
    "corpse", "murder", "kill", "killing", "suicide", "gun", "guns", "rifle", "pistol", "weapon",
    "weapons", "bomb", "terrorist", "drugs", "cocaine", "heroin", "cigarette", "cigarettes",
    "alcohol", "beer", "wine", "vodka", "nazi", "swastika", "rape", "torture", "decapitated",
    "hentai", "fetish", "lingerie",
];

pub fn kid_safe(text: &str) -> bool {
    let lower = text.to_lowercase();
    !lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| BLOCKED.contains(&word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_style_suffix_is_always_on_the_end() {
        for scene in ["a cat in a santa hat", "", "   ", "a cat.", "x, ", &"long ".repeat(500)] {
            let out = finish(scene);
            assert!(out.ends_with(STYLE_SUFFIX), "{out:.80}");
        }
    }

    #[test]
    fn finishing_twice_does_not_repeat_the_suffix() {
        let once = finish("a snowman");
        assert_eq!(finish(&once), once);
        assert_eq!(once.matches(STYLE_SUFFIX).count(), 1);
    }

    #[test]
    fn the_suffix_asks_for_the_whole_colouring_page_style() {
        for want in [
            "black and white",
            "line art",
            "thick clean outlines",
            "no shading",
            "no grey",
            "white background",
            "large closed regions",
            "cute",
            "no text",
        ] {
            assert!(STYLE_SUFFIX.contains(want), "{want}");
        }
    }

    #[test]
    fn a_long_scene_is_cut_before_the_suffix_not_through_it() {
        let out = finish(&"cat ".repeat(1000));
        assert!(out.len() < SCENE_MAX + STYLE_SUFFIX.len() + 8);
        assert!(out.ends_with(STYLE_SUFFIX));
    }

    #[test]
    fn the_inline_negative_names_what_to_avoid() {
        let p = with_negative_inline(&finish("a cat"));
        assert!(p.contains("Do not include") && p.contains("shading"));
    }

    #[test]
    fn unsafe_words_are_caught_by_word_not_substring() {
        assert!(!kid_safe("a gun fight"));
        assert!(!kid_safe("Naked people"));
        assert!(kid_safe("cute christmas cats"));
        assert!(kid_safe("a sussex spaniel in a gunnera patch"));
        assert!(kid_safe("a pirate ship with a treasure map"));
    }
}
