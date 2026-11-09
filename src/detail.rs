use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Detail {
    VerySimple,
    Simple,
    #[default]
    Medium,
    Detailed,
    VeryDetailed,
}

impl Detail {
    pub const ALL: [Detail; 5] = [Detail::VerySimple, Detail::Simple, Detail::Medium, Detail::Detailed, Detail::VeryDetailed];

    pub fn index(self) -> usize {
        Detail::ALL.iter().position(|d| *d == self).unwrap_or(2)
    }

    pub fn from_index(index: usize) -> Detail {
        Detail::ALL[index.min(Detail::ALL.len() - 1)]
    }

    pub fn label(self) -> &'static str {
        match self {
            Detail::VerySimple => "very simple",
            Detail::Simple => "simple",
            Detail::Medium => "medium",
            Detail::Detailed => "detailed",
            Detail::VeryDetailed => "very detailed",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Detail::VerySimple => "very simple drawing, one single big subject with very bold thick outlines, only a handful of huge shapes, no background at all",
            Detail::Simple => "simple drawing, one main subject with thick outlines, big easy shapes, a few clear details, a plain empty background",
            Detail::Medium => "medium detail drawing, a clear main subject with some details and patterns, a few objects around it, a simple background, medium outlines",
            Detail::Detailed => "detailed drawing, a rich scene with many details, patterns on the objects, several objects and a full background, medium and small regions, fine outlines",
            Detail::VeryDetailed => "very detailed intricate drawing, a busy scene packed with fine patterns and decorations, thin precise line work, a full detailed background, many small regions, like a colouring page for older children and adults",
        }
    }

    pub fn brief(self) -> &'static str {
        match self {
            Detail::VerySimple => "very simple: one big subject, no background, a handful of huge shapes",
            Detail::Simple => "simple: one main subject, big shapes, a few clear details, plain background",
            Detail::Medium => "medium: a main subject with some details and a few objects around it",
            Detail::Detailed => "detailed: many details, patterns on objects, a full background with several objects",
            Detail::VeryDetailed => "very detailed: intricate, packed with fine patterns, busy full background, many small regions",
        }
    }

    pub fn avoid(self) -> &'static str {
        match self {
            Detail::VerySimple => "tiny details, small shapes, texture, background, clutter",
            Detail::Simple => "tiny details, small shapes, texture, clutter",
            Detail::Medium | Detail::Detailed | Detail::VeryDetailed => "",
        }
    }

    pub fn speck(self) -> f32 {
        match self {
            Detail::VerySimple => 1.5,
            Detail::Simple => 1.0,
            Detail::Medium => 0.6,
            Detail::Detailed => 0.3,
            Detail::VeryDetailed => 0.1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Look {
    pub detail: Detail,
    pub close: bool,
}

impl Look {
    pub fn denoise(self) -> f32 {
        const CLOSE: [f32; 5] = [0.65, 0.62, 0.60, 0.57, 0.55];
        const INSPIRED: [f32; 5] = [0.75, 0.73, 0.71, 0.69, 0.67];
        let table = if self.close { CLOSE } else { INSPIRED };
        table[self.detail.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_five_levels_that_round_trip_through_their_index() {
        assert_eq!(Detail::ALL.len(), 5);
        for (i, d) in Detail::ALL.iter().enumerate() {
            assert_eq!(Detail::from_index(d.index()), *d);
            assert_eq!(d.index(), i);
        }
        assert_eq!(Detail::from_index(99), Detail::VeryDetailed);
    }

    #[test]
    fn each_level_has_its_own_wording() {
        let mut seen: Vec<&str> = Detail::ALL.iter().map(|d| d.words()).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 5);
    }

    #[test]
    fn the_reference_denoise_stays_in_range_and_follows_the_choices() {
        for close in [true, false] {
            let mut last = f32::MAX;
            for detail in Detail::ALL {
                let d = Look { detail, close }.denoise();
                assert!((0.55..=0.75).contains(&d), "{d}");
                assert!(d < last, "more detail keeps more of the picture");
                last = d;
            }
        }
        for detail in Detail::ALL {
            assert!(Look { detail, close: true }.denoise() < Look { detail, close: false }.denoise());
        }
    }

    #[test]
    fn speck_removal_gets_gentler_with_detail() {
        let s: Vec<f32> = Detail::ALL.iter().map(|d| d.speck()).collect();
        assert!(s.windows(2).all(|w| w[0] > w[1]));
    }

    #[test]
    fn tiny_details_are_only_avoided_at_the_low_levels() {
        assert!(Detail::VerySimple.avoid().contains("tiny details"));
        assert!(Detail::Simple.avoid().contains("tiny details"));
        for d in [Detail::Medium, Detail::Detailed, Detail::VeryDetailed] {
            assert!(!d.avoid().contains("tiny details"));
        }
    }
}
