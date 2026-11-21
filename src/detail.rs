use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Detail {
    VerySimple,
    Simple,
    #[default]
    Medium,
    Detailed,
    VeryDetailed,
    Intricate,
}

impl Detail {
    pub const ALL: [Detail; 6] =
        [Detail::VerySimple, Detail::Simple, Detail::Medium, Detail::Detailed, Detail::VeryDetailed, Detail::Intricate];

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
            Detail::Intricate => "intricate",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Detail::VerySimple => "very simple drawing, one single big subject with very bold thick outlines, only a handful of huge shapes, big simple shapes, no background at all",
            Detail::Simple => "simple drawing, one main subject with thick outlines, big simple shapes, a few clear details, a plain empty background",
            Detail::Medium => "medium detail drawing, a clear main subject with some details and patterns, a few objects around it, a modest background, medium outlines",
            Detail::Detailed => "detailed drawing, a rich scene with many details, patterns on the objects, several objects and a full background, medium and small regions, fine outlines",
            Detail::VeryDetailed => "very detailed drawing, a busy scene packed with fine patterns and decorations, thin precise line work, a full detailed background, many small regions",
            Detail::Intricate => "intricate ornate drawing, dense mandala and zentangle style patterns filling the whole page, very fine thin lines, a great many tiny regions, elaborate decoration everywhere",
        }
    }

    pub fn brief(self) -> &'static str {
        match self {
            Detail::VerySimple => "very simple: one big subject, no background, a handful of huge shapes",
            Detail::Simple => "simple: one main subject, big shapes, a few clear details, plain background",
            Detail::Medium => "medium: a main subject with some details and a few objects around it",
            Detail::Detailed => "detailed: many details, patterns on objects, a full background with several objects",
            Detail::VeryDetailed => "very detailed: packed with fine patterns, busy full background, many small regions",
            Detail::Intricate => "intricate: dense mandala or zentangle style patterns, very fine lines, a great many tiny regions",
        }
    }

    pub fn avoid(self) -> &'static str {
        match self {
            Detail::VerySimple => "tiny details, small shapes, texture, background, clutter",
            Detail::Simple => "tiny details, small shapes, texture, clutter",
            Detail::Medium | Detail::Detailed | Detail::VeryDetailed | Detail::Intricate => "",
        }
    }

    pub fn speck(self) -> f32 {
        match self {
            Detail::VerySimple => 1.5,
            Detail::Simple => 1.0,
            Detail::Medium => 0.6,
            Detail::Detailed => 0.3,
            Detail::VeryDetailed => 0.1,
            Detail::Intricate => 0.04,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Age {
    Years(u8),
    Teen,
    Adult,
}

impl Default for Age {
    fn default() -> Self {
        Age::Years(7)
    }
}

impl Age {
    pub const COUNT: usize = 13;
    pub const MIN_YEARS: u8 = 2;
    pub const MAX_YEARS: u8 = 12;

    pub fn years(n: u8) -> Age {
        Age::Years(n.clamp(Age::MIN_YEARS, Age::MAX_YEARS))
    }

    pub fn index(self) -> usize {
        match self {
            Age::Years(n) => (n.clamp(Age::MIN_YEARS, Age::MAX_YEARS) - Age::MIN_YEARS) as usize,
            Age::Teen => Age::COUNT - 2,
            Age::Adult => Age::COUNT - 1,
        }
    }

    pub fn from_index(index: usize) -> Age {
        match index.min(Age::COUNT - 1) {
            11 => Age::Teen,
            12 => Age::Adult,
            i => Age::Years(i as u8 + Age::MIN_YEARS),
        }
    }

    pub fn label(self) -> String {
        match self {
            Age::Years(n) => format!("age {n}"),
            Age::Teen => "teen".into(),
            Age::Adult => "adult".into(),
        }
    }

    pub fn from_detail(detail: Detail) -> Age {
        match detail {
            Detail::VerySimple => Age::Years(3),
            Detail::Simple => Age::Years(5),
            Detail::Medium => Age::Years(7),
            Detail::Detailed => Age::Years(10),
            Detail::VeryDetailed => Age::Teen,
            Detail::Intricate => Age::Adult,
        }
    }

    pub fn detail(self) -> Detail {
        match self {
            Age::Years(n) if n <= 3 => Detail::VerySimple,
            Age::Years(n) if n <= 5 => Detail::Simple,
            Age::Years(n) if n <= 7 => Detail::Medium,
            Age::Years(n) if n <= 10 => Detail::Detailed,
            Age::Years(_) | Age::Teen => Detail::VeryDetailed,
            Age::Adult => Detail::Intricate,
        }
    }

    pub fn tone(self) -> &'static str {
        match self {
            Age::Years(n) if n <= 5 => "friendly cute round characters with big heads and soft rounded shapes",
            Age::Years(n) if n <= 8 => "friendly characters with clear shapes and a playful feel",
            Age::Years(_) => "realistic proportions, dynamic action poses and an adventurous scene",
            Age::Teen => "stylish and sophisticated, realistic proportions, expressive dynamic composition, graphic illustration style",
            Age::Adult => "sophisticated fine art illustration, elegant realistic proportions, rich ornamental composition",
        }
    }

    pub fn words(self) -> String {
        format!("{}, {}", self.detail().words(), self.tone())
    }

    pub fn audience(self) -> String {
        match self {
            Age::Years(n) if n <= 5 => format!("a {n} year old: big simple shapes, one friendly subject, cute and round, nothing fiddly"),
            Age::Years(n) if n <= 8 => format!("a {n} year old: clear shapes, a friendly subject, a few objects"),
            Age::Years(n) => format!("a {n} year old: realistic proportions, dynamic scenes, real detail, nothing babyish"),
            Age::Teen => "a teenager: sophisticated, stylish, realistic proportions, dynamic and detailed, nothing childish".into(),
            Age::Adult => "an adult: sophisticated and intricate, fine detail, mandala or zentangle style patterns, nothing childish".into(),
        }
    }

    pub fn brief(self) -> String {
        format!("{}. Colourer: {}. Detail: {}", self.label(), self.audience(), self.detail().brief())
    }

    pub fn avoid(self) -> &'static str {
        self.detail().avoid()
    }

    pub fn speck(self) -> f32 {
        self.detail().speck()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Default)]
pub struct Look {
    pub age: Age,
    pub close: bool,
}

#[derive(Deserialize)]
struct LookWire {
    age: Option<Age>,
    detail: Option<Detail>,
    #[serde(default)]
    close: bool,
}

impl<'de> Deserialize<'de> for Look {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = LookWire::deserialize(d)?;
        let age = wire.age.or(wire.detail.map(Age::from_detail)).unwrap_or_default();
        Ok(Look { age, close: wire.close })
    }
}

impl Look {
    pub fn detail(self) -> Detail {
        self.age.detail()
    }

    pub fn denoise(self) -> f32 {
        const CLOSE: [f32; 6] = [0.65, 0.62, 0.60, 0.57, 0.55, 0.52];
        const INSPIRED: [f32; 6] = [0.75, 0.73, 0.71, 0.69, 0.67, 0.64];
        let table = if self.close { CLOSE } else { INSPIRED };
        table[self.detail().index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(detail: Detail, close: bool) -> Look {
        Look { age: Age::from_detail(detail), close }
    }

    #[test]
    fn there_are_six_levels_that_round_trip_through_their_index() {
        assert_eq!(Detail::ALL.len(), 6);
        for (i, d) in Detail::ALL.iter().enumerate() {
            assert_eq!(Detail::from_index(d.index()), *d);
            assert_eq!(d.index(), i);
        }
        assert_eq!(Detail::from_index(99), Detail::Intricate);
    }

    #[test]
    fn each_level_has_its_own_wording() {
        let mut seen: Vec<&str> = Detail::ALL.iter().map(|d| d.words()).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 6);
    }

    #[test]
    fn ages_run_from_two_to_twelve_then_teen_and_adult() {
        assert_eq!(Age::COUNT, 13);
        for i in 0..Age::COUNT {
            assert_eq!(Age::from_index(i).index(), i);
        }
        assert_eq!(Age::from_index(0), Age::Years(2));
        assert_eq!(Age::from_index(10), Age::Years(12));
        assert_eq!(Age::from_index(11), Age::Teen);
        assert_eq!(Age::from_index(99), Age::Adult);
        assert_eq!(Age::years(1), Age::Years(2));
        assert_eq!(Age::years(40), Age::Years(12));
    }

    #[test]
    fn older_ages_never_get_less_detail() {
        let mut last = 0;
        for i in 0..Age::COUNT {
            let d = Age::from_index(i).detail().index();
            assert!(d >= last);
            last = d;
        }
        assert_eq!(Age::Years(3).detail(), Detail::VerySimple);
        assert_eq!(Age::Adult.detail(), Detail::Intricate);
    }

    #[test]
    fn old_detail_levels_map_to_an_age_with_the_same_level_or_close() {
        for d in Detail::ALL {
            assert_eq!(Age::from_detail(d).detail(), d);
        }
    }

    #[test]
    fn the_reference_denoise_stays_in_range_and_follows_the_choices() {
        for close in [true, false] {
            let mut last = f32::MAX;
            for detail in Detail::ALL {
                let d = look(detail, close).denoise();
                assert!((0.5..=0.75).contains(&d), "{d}");
                assert!(d < last, "more detail keeps more of the picture");
                last = d;
            }
        }
        for detail in Detail::ALL {
            assert!(look(detail, true).denoise() < look(detail, false).denoise());
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
        for d in [Detail::Medium, Detail::Detailed, Detail::VeryDetailed, Detail::Intricate] {
            assert!(!d.avoid().contains("tiny details"));
        }
    }

    #[test]
    fn a_look_without_an_age_reads_its_old_detail() {
        let l: Look = serde_json::from_str(r#"{"detail":"VeryDetailed","close":true}"#).unwrap();
        assert_eq!(l, Look { age: Age::Teen, close: true });
        let l: Look = serde_json::from_str(r#"{"age":{"Years":4},"close":false}"#).unwrap();
        assert_eq!(l.age, Age::Years(4));
        let l: Look = serde_json::from_str("{}").unwrap();
        assert_eq!(l, Look::default());
    }
}
