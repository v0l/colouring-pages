use crate::llm::{Llm, json_object};
use crate::style::kid_safe;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PAGES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AgeBand {
    Toddler,
    #[default]
    Young,
    Older,
}

impl AgeBand {
    pub fn parse(text: &str) -> AgeBand {
        let t = text.to_lowercase();
        if t.contains("toddler") || t.contains("2-4") || t.contains("2 to 4") || t.contains("3-5") {
            AgeBand::Toddler
        } else if t.contains("older") || t.contains("8-") || t.contains("9-") || t.contains("10") {
            AgeBand::Older
        } else {
            AgeBand::Young
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgeBand::Toddler => "ages 2 to 4",
            AgeBand::Young => "ages 5 to 7",
            AgeBand::Older => "ages 8 to 10",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            AgeBand::Toddler => "very few, very big shapes, one main subject, nothing small",
            AgeBand::Young => "big simple shapes, a few clear details",
            AgeBand::Older => "more detail and patterns, still clean closed outlines",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub subject: String,
    pub setting: String,
    pub mood: String,
    pub ideas: Vec<String>,
    pub age: AgeBand,
    pub softened: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlanError {
    Refused(String),
    Unreadable(String),
    Failed(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::Refused(why) => write!(f, "{why}"),
            PlanError::Unreadable(why) | PlanError::Failed(why) => write!(f, "{why}"),
        }
    }
}

pub const REFUSAL: &str = "Let's colour something else. How about animals, space or a castle?";

pub const PLAN_SYSTEM: &str = r#"You help a child get printable colouring pages. The child's request was spoken, so it may be badly transcribed. Turn it into a plan.

Reply with one JSON object and nothing else:
{
  "safe": true,
  "reason": "",
  "subject": "main thing, a few words",
  "setting": "where it happens",
  "mood": "one or two words",
  "age": "toddler | young | older",
  "softened": "",
  "ideas": ["idea 1", "idea 2", "idea 3", "idea 4"]
}

Rules:
- Exactly 4 ideas. Each is ONE simple scene a child can colour in, one sentence, concrete, no text or words in the picture. The 4 scenes must differ from each other.
- Everything must be suitable for children aged 2 to 10. If the request is only partly suitable, drop the unsuitable part, make the ideas friendly, and say what you changed in "softened".
- If nothing suitable can be made (violence, weapons, nudity, drugs, hate, anything scary or adult), set "safe" to false, give a short kind "reason" for a child, and leave ideas empty.
- "age": toddler (2 to 4) for very simple requests, young (5 to 7) by default, older (8 to 10) when the request asks for detail or a complex subject."#;

pub const PROMPT_SYSTEM: &str = r#"You write image prompts for a colouring page generator. You get a list of scene ideas. For each idea write one prompt describing only the scene: the subject, what it is doing, a few simple objects around it, a simple background. Use plain words. Do not describe colours, shading or style, that is added later. No text or letters in the picture.

Reply with one JSON object and nothing else:
{ "prompts": ["prompt for idea 1", "prompt for idea 2", ...] }

Exactly one prompt per idea, in the same order."#;

pub fn plan_request(request: &str) -> String {
    format!("The child said: \"{}\"", request.trim().replace('"', "'"))
}

pub fn parse_plan(raw: &str, request: &str) -> Result<Plan, PlanError> {
    let v = json_object(raw).ok_or_else(|| PlanError::Unreadable("the plan was not JSON".into()))?;
    let v = match v {
        Value::Array(_) => return Err(PlanError::Unreadable("the plan was a list".into())),
        v => v,
    };
    let safe = match v.get("safe") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !matches!(s.trim().to_lowercase().as_str(), "false" | "no"),
        _ => true,
    };
    if !safe {
        let why = text(&v, "reason");
        return Err(PlanError::Refused(if why.is_empty() { REFUSAL.into() } else { why }));
    }
    let ideas: Vec<String> = match v.get("ideas") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|i| match i {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Object(o) => o
                    .get("idea")
                    .or_else(|| o.get("scene"))
                    .or_else(|| o.get("description"))
                    .and_then(|s| s.as_str())
                    .map(|s| s.trim().to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    };
    let subject = {
        let s = text(&v, "subject");
        if s.is_empty() { request.trim().to_string() } else { s }
    };
    let setting = text(&v, "setting");
    let mood = text(&v, "mood");
    let ideas: Vec<String> = ideas.into_iter().filter(|i| kid_safe(i)).collect();
    if ideas.is_empty() {
        return Err(PlanError::Unreadable("the plan had no usable ideas".into()));
    }
    let softened = Some(text(&v, "softened")).filter(|s| !s.is_empty());
    Ok(Plan {
        ideas: fit_ideas(ideas, &subject),
        subject,
        setting,
        mood,
        age: AgeBand::parse(&text(&v, "age")),
        softened,
    })
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(|s| s.as_str()).unwrap_or_default().trim().to_string()
}

const ANGLES: [&str; 4] = ["", ", close up", ", seen from far away", ", with a friend"];

pub fn fit_ideas(mut ideas: Vec<String>, subject: &str) -> Vec<String> {
    ideas.truncate(PAGES);
    let base = ideas.clone();
    let mut round = 0;
    while ideas.len() < PAGES {
        round += 1;
        let from = &base[(ideas.len() - base.len()) % base.len()];
        let angle = ANGLES[round % ANGLES.len()];
        let angle = if angle.is_empty() { ", another view" } else { angle };
        let idea = from.trim_end_matches('.');
        ideas.push(if idea.is_empty() { format!("{subject}{angle}") } else { format!("{idea}{angle}") });
    }
    ideas
}

pub fn parse_prompts(raw: &str, ideas: &[String]) -> Result<Vec<String>, String> {
    let v = json_object(raw).ok_or_else(|| "the prompts were not JSON".to_string())?;
    let list = match &v {
        Value::Array(a) => Some(a),
        Value::Object(o) => o
            .get("prompts")
            .or_else(|| o.get("pages"))
            .and_then(|p| p.as_array()),
        _ => None,
    }
    .ok_or_else(|| "the prompts had no list".to_string())?;
    let got: Vec<String> = list
        .iter()
        .map(|p| match p {
            Value::String(s) => s.trim().to_string(),
            Value::Object(o) => o
                .get("prompt")
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .trim()
                .to_string(),
            _ => String::new(),
        })
        .collect();
    if got.iter().all(|p| p.is_empty()) {
        return Err("the prompts were empty".into());
    }
    Ok(ideas
        .iter()
        .enumerate()
        .map(|(i, idea)| match got.get(i) {
            Some(p) if !p.is_empty() && kid_safe(p) => crate::style::finish(p),
            _ => crate::style::finish(idea),
        })
        .collect())
}

pub fn prompts_request(plan: &Plan) -> String {
    let mut out = format!(
        "Subject: {}\nSetting: {}\nMood: {}\nFor children {}: {}.\nIdeas:\n",
        plan.subject,
        plan.setting,
        plan.mood,
        plan.age.label(),
        plan.age.detail()
    );
    for (i, idea) in plan.ideas.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, idea));
    }
    out
}

pub async fn make_plan(llm: &Llm, request: &str) -> Result<Plan, PlanError> {
    if !kid_safe(request) {
        return Err(PlanError::Refused(REFUSAL.into()));
    }
    let mut last = String::new();
    for _ in 0..2 {
        let raw = llm.chat(PLAN_SYSTEM, &plan_request(request)).await.map_err(PlanError::Failed)?;
        match parse_plan(&raw, request) {
            Ok(plan) => return Ok(plan),
            Err(PlanError::Unreadable(why)) => last = why,
            Err(e) => return Err(e),
        }
    }
    Err(PlanError::Unreadable(last))
}

pub async fn make_prompts(llm: &Llm, plan: &Plan) -> Vec<String> {
    for _ in 0..2 {
        let Ok(raw) = llm.chat(PROMPT_SYSTEM, &prompts_request(plan)).await else { continue };
        if let Ok(p) = parse_prompts(&raw, &plan.ideas) {
            return p;
        }
    }
    plan.ideas.iter().map(|i| crate::style::finish(i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Reply, serve};
    use crate::style::STYLE_SUFFIX;
    use serde_json::json;

    const GOOD: &str = r#"{"safe":true,"reason":"","subject":"christmas cats","setting":"a cosy living room","mood":"cheerful","age":"young","softened":"","ideas":["A kitten hanging a bauble on a tree","A cat wearing a santa hat","Two cats wrapping a present","A cat asleep by the fireplace"]}"#;

    fn ideas() -> Vec<String> {
        ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_good_plan_is_read() {
        let p = parse_plan(GOOD, "cute christmas cats").unwrap();
        assert_eq!(p.subject, "christmas cats");
        assert_eq!(p.ideas.len(), 4);
        assert_eq!(p.age, AgeBand::Young);
        assert!(p.softened.is_none());
    }

    #[test]
    fn a_plan_in_a_fence_with_chatter_is_read() {
        let raw = format!("Here is the plan:\n```json\n{GOOD}\n```\nHope that helps!");
        assert_eq!(parse_plan(&raw, "x").unwrap().ideas.len(), 4);
    }

    #[test]
    fn a_plan_missing_fields_still_gets_four_ideas() {
        let p = parse_plan(r#"{"ideas":["one cat","two cats"]}"#, "cats").unwrap();
        assert_eq!(p.ideas.len(), PAGES);
        assert_eq!(p.subject, "cats");
        assert_eq!(p.age, AgeBand::Young);
        assert_eq!(p.ideas[0], "one cat");
        assert_ne!(p.ideas[2], p.ideas[0]);
    }

    #[test]
    fn too_many_ideas_are_cut_to_four() {
        let p = parse_plan(r#"{"ideas":["1","2","3","4","5","6"]}"#, "x").unwrap();
        assert_eq!(p.ideas, vec!["1", "2", "3", "4"]);
    }

    #[test]
    fn ideas_may_be_objects() {
        let p = parse_plan(r#"{"ideas":[{"idea":"a cat"},{"scene":"a dog"},{"nope":1},7]}"#, "x").unwrap();
        assert_eq!(p.ideas[0], "a cat");
        assert_eq!(p.ideas[1], "a dog");
        assert_eq!(p.ideas.len(), 4);
    }

    #[test]
    fn malformed_plans_are_errors_not_panics() {
        for raw in ["", "I cannot do that", "{", "[1,2,3]", r#"{"ideas":"a cat"}"#, r#"{"ideas":[]}"#, r#"{"ideas":[1,2]}"#, "null"] {
            assert!(matches!(parse_plan(raw, "x"), Err(PlanError::Unreadable(_))), "{raw}");
        }
    }

    #[test]
    fn an_unsafe_plan_is_refused_with_the_reason() {
        let raw = r#"{"safe":false,"reason":"Let's draw something friendly instead.","ideas":[]}"#;
        assert_eq!(
            parse_plan(raw, "x"),
            Err(PlanError::Refused("Let's draw something friendly instead.".into()))
        );
        let raw = r#"{"safe":"false","ideas":["a"]}"#;
        assert_eq!(parse_plan(raw, "x"), Err(PlanError::Refused(REFUSAL.into())));
    }

    #[test]
    fn unsafe_ideas_are_dropped_even_if_the_model_says_safe() {
        let raw = r#"{"safe":true,"ideas":["a gun fight","a happy dog"]}"#;
        let p = parse_plan(raw, "x").unwrap();
        assert!(p.ideas.iter().all(|i| !i.contains("gun")));
        assert!(p.ideas[0].contains("happy dog"));
        let raw = r#"{"safe":true,"ideas":["a gun fight"]}"#;
        assert!(parse_plan(raw, "x").is_err());
    }

    #[test]
    fn the_age_band_is_read_from_loose_words() {
        assert_eq!(AgeBand::parse("toddler"), AgeBand::Toddler);
        assert_eq!(AgeBand::parse("Older kids"), AgeBand::Older);
        assert_eq!(AgeBand::parse("young"), AgeBand::Young);
        assert_eq!(AgeBand::parse(""), AgeBand::Young);
    }

    #[test]
    fn prompts_always_end_with_the_style_suffix() {
        let raw = r#"{"prompts":["a cat with a hat","a dog","","a bird"]}"#;
        let p = parse_prompts(raw, &ideas()).unwrap();
        assert_eq!(p.len(), 4);
        assert!(p.iter().all(|s| s.ends_with(STYLE_SUFFIX)));
        assert!(p[0].starts_with("a cat with a hat"));
        assert!(p[2].starts_with("c."), "an empty prompt falls back to the idea: {}", p[2]);
    }

    #[test]
    fn prompts_may_be_a_bare_list_or_objects_and_short_lists_are_filled() {
        let p = parse_prompts(r#"["x","y"]"#, &ideas()).unwrap();
        assert_eq!(p.len(), 4);
        assert!(p[3].starts_with("d."));
        let p = parse_prompts(r#"{"pages":[{"prompt":"one"},{"prompt":"two"}]}"#, &ideas()).unwrap();
        assert!(p[1].starts_with("two"));
        assert!(p.iter().all(|s| s.ends_with(STYLE_SUFFIX)));
    }

    #[test]
    fn malformed_prompts_are_errors() {
        for raw in ["", "nothing", "{}", r#"{"prompts":"a"}"#, r#"{"prompts":[]}"#, r#"{"prompts":["",""]}"#, "{"] {
            assert!(parse_prompts(raw, &ideas()).is_err(), "{raw}");
        }
    }

    #[test]
    fn an_unsafe_prompt_falls_back_to_its_idea() {
        let p = parse_prompts(r#"{"prompts":["a gun","b","c","d"]}"#, &ideas()).unwrap();
        assert!(p[0].starts_with("a."));
    }

    fn llm_returning(answers: Vec<String>) -> (crate::fake::Fake, Llm) {
        let n = std::sync::atomic::AtomicUsize::new(0);
        let fake = serve(move |_| {
            let i = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let a = answers[i.min(answers.len() - 1)].clone();
            Reply::json(json!({"choices":[{"message":{"content":a}}]}))
        });
        let llm = Llm { url: fake.url.clone(), key: String::new(), model: "m".into() };
        (fake, llm)
    }

    #[test]
    fn make_plan_retries_once_on_garbage() {
        let (fake, llm) = llm_returning(vec!["sorry".into(), GOOD.into()]);
        let p = crate::net::runtime().block_on(make_plan(&llm, "cute christmas cats")).unwrap();
        assert_eq!(p.ideas.len(), 4);
        assert_eq!(fake.requests().len(), 2);
    }

    #[test]
    fn make_plan_gives_up_after_two_garbage_answers() {
        let (fake, llm) = llm_returning(vec!["sorry".into()]);
        let e = crate::net::runtime().block_on(make_plan(&llm, "cats")).unwrap_err();
        assert!(matches!(e, PlanError::Unreadable(_)));
        assert_eq!(fake.requests().len(), 2);
    }

    #[test]
    fn an_unsafe_request_never_reaches_the_model() {
        let (fake, llm) = llm_returning(vec![GOOD.into()]);
        let e = crate::net::runtime().block_on(make_plan(&llm, "a gun")).unwrap_err();
        assert!(matches!(e, PlanError::Refused(_)));
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn make_prompts_falls_back_to_the_ideas_when_the_model_fails() {
        let (_fake, llm) = llm_returning(vec!["nonsense".into()]);
        let plan = parse_plan(GOOD, "x").unwrap();
        let p = crate::net::runtime().block_on(make_prompts(&llm, &plan));
        assert_eq!(p.len(), 4);
        assert!(p.iter().all(|s| s.ends_with(STYLE_SUFFIX)));
        assert!(p[0].starts_with("A kitten hanging a bauble"));
    }

    #[test]
    fn the_request_is_quoted_safely() {
        assert_eq!(plan_request(" say \"hi\" "), "The child said: \"say 'hi'\"");
    }
}
