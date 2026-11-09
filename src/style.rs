use crate::detail::Detail;

pub const STYLE_SUFFIX: &str = "black and white line art colouring page for children, clean closed outlines, no shading, no grey, no filled black areas, pure white background, no text, no letters";

const NEGATIVE_BASE: &str = "shading, grey, gray, gradient, colour, color, filled black areas, hatching, crosshatch, text, letters, numbers, watermark, signature, photo, realistic";

pub fn negative(detail: Detail) -> String {
    match detail.avoid() {
        "" => NEGATIVE_BASE.to_string(),
        more => format!("{NEGATIVE_BASE}, {more}"),
    }
}

const SCENE_MAX: usize = 700;

fn trim_joiners(text: &str) -> &str {
    text.trim().trim_end_matches(['.', ',', ';', ':', ' '])
}

pub fn finish(scene: &str, detail: Detail) -> String {
    let scene = scene.trim();
    let scene = scene.strip_suffix(STYLE_SUFFIX).unwrap_or(scene);
    let scene = trim_joiners(scene);
    let scene = trim_joiners(scene.strip_suffix(detail.words()).unwrap_or(scene));
    let scene: String = scene.chars().take(SCENE_MAX).collect();
    let scene = trim_joiners(&scene);
    if scene.is_empty() {
        format!("{}. {STYLE_SUFFIX}", detail.words())
    } else {
        format!("{scene}. {}. {STYLE_SUFFIX}", detail.words())
    }
}

pub fn with_negative_inline(prompt: &str, detail: Detail) -> String {
    format!("{prompt}. Do not include: {}.", negative(detail))
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
        for detail in Detail::ALL {
            for scene in ["a cat in a santa hat", "", "   ", "a cat.", "x, ", &"long ".repeat(500)] {
                let out = finish(scene, detail);
                assert!(out.ends_with(STYLE_SUFFIX), "{out:.80}");
                assert!(out.contains(detail.words()));
            }
        }
    }

    #[test]
    fn finishing_twice_does_not_repeat_the_suffix() {
        for detail in Detail::ALL {
            let once = finish("a snowman", detail);
            assert_eq!(finish(&once, detail), once);
            assert_eq!(once.matches(STYLE_SUFFIX).count(), 1);
            assert_eq!(once.matches(detail.words()).count(), 1);
        }
    }

    #[test]
    fn the_wording_changes_with_the_level_but_the_invariants_do_not() {
        let low = finish("a cat", Detail::VerySimple);
        let high = finish("a cat", Detail::VeryDetailed);
        assert_ne!(low, high);
        assert!(low.contains("huge shapes") && !high.contains("huge shapes"));
        assert!(high.contains("fine patterns") && !low.contains("fine patterns"));
        for p in [&low, &high] {
            assert!(p.ends_with(STYLE_SUFFIX));
        }
        for gone in ["cute", "large closed regions", "one main subject that fills the page"] {
            assert!(!STYLE_SUFFIX.contains(gone), "{gone}");
        }
    }

    #[test]
    fn the_negative_forbids_tiny_details_only_at_the_low_levels() {
        assert!(negative(Detail::VerySimple).contains("tiny details"));
        assert!(negative(Detail::Simple).contains("tiny details"));
        for d in [Detail::Medium, Detail::Detailed, Detail::VeryDetailed] {
            let n = negative(d);
            assert!(!n.contains("tiny details") && n.contains("shading") && n.contains("grey"), "{n}");
        }
    }

    #[test]
    fn the_suffix_asks_for_the_whole_colouring_page_style() {
        for want in [
            "black and white",
            "line art",
            "clean closed outlines",
            "no shading",
            "no grey",
            "white background",
            "no text",
        ] {
            assert!(STYLE_SUFFIX.contains(want), "{want}");
        }
    }

    #[test]
    fn a_long_scene_is_cut_before_the_suffix_not_through_it() {
        let out = finish(&"cat ".repeat(1000), Detail::Medium);
        assert!(out.len() < SCENE_MAX + Detail::Medium.words().len() + STYLE_SUFFIX.len() + 8);
        assert!(out.ends_with(STYLE_SUFFIX));
    }

    #[test]
    fn the_inline_negative_names_what_to_avoid() {
        let p = with_negative_inline(&finish("a cat", Detail::Medium), Detail::Medium);
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
