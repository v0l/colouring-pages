use crate::detail::{Age, Detail};
use crate::llm::{Llm, json_object};
use crate::style::kid_safe;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PAGES: usize = 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub subject: String,
    pub setting: String,
    pub mood: String,
    pub ideas: Vec<String>,
    #[serde(default)]
    pub detail: Detail,
    #[serde(default)]
    pub age: Age,
    #[serde(default)]
    pub picture: Option<String>,
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

const PLAN_HEAD: &str = r#"You help someone get printable colouring pages. The request was spoken, so it may be badly transcribed. Turn it into a plan.

Reply with one JSON object and nothing else:
{
  "safe": true,
  "reason": "",
  "subject": "main thing, a few words",
  "setting": "where it happens",
  "mood": "one or two words",
  "picture": "only when a picture is attached: what it shows, in one or two sentences (the subject, its pose, the setting, distinctive features); otherwise empty",
  "softened": "",
  "ideas": ["idea 1", "idea 2", "idea 3", "idea 4"]
}

Rules:
- Exactly 4 ideas. Each is ONE scene, one sentence, concrete, no text or words in the picture. The 4 scenes must differ from each other.
- You are told who will colour the pages and how detailed they must be. Write ideas that fit that person and level.
- Everything must stay free of violence, weapons, nudity, drugs, hate and gore, at every age. If the request is only partly acceptable, drop that part, keep the ideas friendly, and say what you changed in "softened".
- If nothing acceptable can be made, set "safe" to false, give a short kind "reason", and leave ideas empty.
- When a picture is attached, it is the heart of the request. Describe it in "picture", and make all 4 ideas variations of THAT picture's subject (a different pose, setting, activity or angle for the same subject, keeping its distinctive features), unless the child's words ask for something else. Do not invent an unrelated scene."#;

pub fn plan_system(age: Age) -> String {
    format!("{PLAN_HEAD}\n\nThe pages are for {}.", age.audience())
}

const PROMPT_HEAD: &str = r#"You write image prompts for a colouring page generator. You get a list of scene ideas. For each idea write one prompt describing only the scene: the subject, what it is doing, and the objects and background that fit the level you are given. Use plain words. Do not describe colours, shading or style, that is added later. No text or letters in the picture.

Reply with one JSON object and nothing else:
{ "prompts": ["prompt for idea 1", "prompt for idea 2", ...] }

Exactly one prompt per idea, in the same order. You are told who will colour the pages and how detailed the scenes must be: fit the subject treatment, the number of objects and the background to that.

When you are told what an attached picture shows, every prompt must name that same subject with its distinctive features and say how this page varies it (the pose, setting or activity from the idea). Never describe a different subject."#;

pub fn prompt_system(age: Age) -> String {
    format!("{PROMPT_HEAD}\n\nThe pages are for {}.", age.audience())
}

pub fn plan_request(request: &str, age: Age, has_picture: bool) -> String {
    let mut out = format!("The person said: \"{}\"\nAudience: {}.", request.trim().replace('"', "'"), age.brief());
    if has_picture {
        out.push_str("\nA picture is attached. Base every idea on it.");
    }
    out
}

pub fn parse_plan(raw: &str, request: &str, age: Age) -> Result<Plan, PlanError> {
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
        detail: age.detail(),
        age,
        picture: Some(text(&v, "picture")).filter(|s| !s.is_empty()),
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

pub fn parse_prompts(raw: &str, ideas: &[String], age: Age) -> Result<Vec<String>, String> {
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
            Some(p) if !p.is_empty() && kid_safe(p) => crate::style::finish(p, age),
            _ => crate::style::finish(idea, age),
        })
        .collect())
}

pub fn prompts_request(plan: &Plan) -> String {
    let mut out = format!(
        "Subject: {}\nSetting: {}\nMood: {}\nAudience: {}.\n",
        plan.subject,
        plan.setting,
        plan.mood,
        plan.age.brief()
    );
    if let Some(picture) = &plan.picture {
        out.push_str(&format!("The attached picture shows: {picture}\n"));
    }
    out.push_str("Ideas:\n");
    for (i, idea) in plan.ideas.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, idea));
    }
    out
}

pub async fn make_plan(llm: &Llm, request: &str, age: Age, picture: Option<&[u8]>) -> Result<Plan, PlanError> {
    if !kid_safe(request) {
        return Err(PlanError::Refused(REFUSAL.into()));
    }
    let mut last = String::new();
    for _ in 0..2 {
        let raw = llm
            .chat_with(&plan_system(age), &plan_request(request, age, picture.is_some()), picture)
            .await
            .map_err(PlanError::Failed)?;
        match parse_plan(&raw, request, age) {
            Ok(plan) => return Ok(plan),
            Err(PlanError::Unreadable(why)) => last = why,
            Err(e) => return Err(e),
        }
    }
    Err(PlanError::Unreadable(last))
}

pub async fn make_prompts(llm: &Llm, plan: &Plan) -> Vec<String> {
    for _ in 0..2 {
        let Ok(raw) = llm.chat(&prompt_system(plan.age), &prompts_request(plan)).await else { continue };
        if let Ok(p) = parse_prompts(&raw, &plan.ideas, plan.age) {
            return p;
        }
    }
    plan.ideas.iter().map(|i| crate::style::finish(i, plan.age)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Reply, serve};
    use crate::style::STYLE_SUFFIX;
    use serde_json::json;

    const GOOD: &str = r#"{"safe":true,"reason":"","subject":"christmas cats","setting":"a cosy living room","mood":"cheerful","picture":"","softened":"","ideas":["A kitten hanging a bauble on a tree","A cat wearing a santa hat","Two cats wrapping a present","A cat asleep by the fireplace"]}"#;

    const M: Age = Age::Years(7);

    fn ideas() -> Vec<String> {
        ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_good_plan_is_read() {
        let p = parse_plan(GOOD, "cute christmas cats", M).unwrap();
        assert_eq!(p.subject, "christmas cats");
        assert_eq!(p.ideas.len(), 4);
        assert_eq!(p.age, M);
        assert!(p.softened.is_none());
    }

    #[test]
    fn a_plan_in_a_fence_with_chatter_is_read() {
        let raw = format!("Here is the plan:\n```json\n{GOOD}\n```\nHope that helps!");
        assert_eq!(parse_plan(&raw, "x", M).unwrap().ideas.len(), 4);
    }

    #[test]
    fn a_plan_missing_fields_still_gets_four_ideas() {
        let p = parse_plan(r#"{"ideas":["one cat","two cats"]}"#, "cats", M).unwrap();
        assert_eq!(p.ideas.len(), PAGES);
        assert_eq!(p.subject, "cats");
        assert_eq!(p.age, M);
        assert_eq!(p.ideas[0], "one cat");
        assert_ne!(p.ideas[2], p.ideas[0]);
    }

    #[test]
    fn too_many_ideas_are_cut_to_four() {
        let p = parse_plan(r#"{"ideas":["1","2","3","4","5","6"]}"#, "x", M).unwrap();
        assert_eq!(p.ideas, vec!["1", "2", "3", "4"]);
    }

    #[test]
    fn ideas_may_be_objects() {
        let p = parse_plan(r#"{"ideas":[{"idea":"a cat"},{"scene":"a dog"},{"nope":1},7]}"#, "x", M).unwrap();
        assert_eq!(p.ideas[0], "a cat");
        assert_eq!(p.ideas[1], "a dog");
        assert_eq!(p.ideas.len(), 4);
    }

    #[test]
    fn malformed_plans_are_errors_not_panics() {
        for raw in ["", "I cannot do that", "{", "[1,2,3]", r#"{"ideas":"a cat"}"#, r#"{"ideas":[]}"#, r#"{"ideas":[1,2]}"#, "null"] {
            assert!(matches!(parse_plan(raw, "x", M), Err(PlanError::Unreadable(_))), "{raw}");
        }
    }

    #[test]
    fn an_unsafe_plan_is_refused_with_the_reason() {
        let raw = r#"{"safe":false,"reason":"Let's draw something friendly instead.","ideas":[]}"#;
        assert_eq!(
            parse_plan(raw, "x", M),
            Err(PlanError::Refused("Let's draw something friendly instead.".into()))
        );
        let raw = r#"{"safe":"false","ideas":["a"]}"#;
        assert_eq!(parse_plan(raw, "x", M), Err(PlanError::Refused(REFUSAL.into())));
    }

    #[test]
    fn unsafe_ideas_are_dropped_even_if_the_model_says_safe() {
        let raw = r#"{"safe":true,"ideas":["a gun fight","a happy dog"]}"#;
        let p = parse_plan(raw, "x", M).unwrap();
        assert!(p.ideas.iter().all(|i| !i.contains("gun")));
        assert!(p.ideas[0].contains("happy dog"));
        let raw = r#"{"safe":true,"ideas":["a gun fight"]}"#;
        assert!(parse_plan(raw, "x", M).is_err());
    }

    #[test]
    fn prompts_always_end_with_the_style_suffix() {
        let raw = r#"{"prompts":["a cat with a hat","a dog","","a bird"]}"#;
        let p = parse_prompts(raw, &ideas(), M).unwrap();
        assert_eq!(p.len(), 4);
        assert!(p.iter().all(|s| s.ends_with(STYLE_SUFFIX)));
        assert!(p[0].starts_with("a cat with a hat"));
        assert!(p[2].starts_with("c."), "an empty prompt falls back to the idea: {}", p[2]);
    }

    #[test]
    fn prompts_may_be_a_bare_list_or_objects_and_short_lists_are_filled() {
        let p = parse_prompts(r#"["x","y"]"#, &ideas(), M).unwrap();
        assert_eq!(p.len(), 4);
        assert!(p[3].starts_with("d."));
        let p = parse_prompts(r#"{"pages":[{"prompt":"one"},{"prompt":"two"}]}"#, &ideas(), M).unwrap();
        assert!(p[1].starts_with("two"));
        assert!(p.iter().all(|s| s.ends_with(STYLE_SUFFIX)));
    }

    #[test]
    fn malformed_prompts_are_errors() {
        for raw in ["", "nothing", "{}", r#"{"prompts":"a"}"#, r#"{"prompts":[]}"#, r#"{"prompts":["",""]}"#, "{"] {
            assert!(parse_prompts(raw, &ideas(), M).is_err(), "{raw}");
        }
    }

    #[test]
    fn an_unsafe_prompt_falls_back_to_its_idea() {
        let p = parse_prompts(r#"{"prompts":["a gun","b","c","d"]}"#, &ideas(), M).unwrap();
        assert!(p[0].starts_with("a."));
    }

    fn llm_returning(answers: Vec<String>) -> (crate::fake::Fake, Llm) {
        let n = std::sync::atomic::AtomicUsize::new(0);
        let fake = serve(move |_| {
            let i = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let a = answers[i.min(answers.len() - 1)].clone();
            Reply::json(json!({"choices":[{"message":{"content":a}}]}))
        });
        let llm = Llm { url: fake.url.clone(), key: String::new(), model: "m".into(), vision_model: String::new() };
        (fake, llm)
    }

    #[test]
    fn make_plan_retries_once_on_garbage() {
        let (fake, llm) = llm_returning(vec!["sorry".into(), GOOD.into()]);
        let p = crate::net::runtime().block_on(make_plan(&llm, "cute christmas cats", M, None)).unwrap();
        assert_eq!(p.ideas.len(), 4);
        assert_eq!(fake.requests().len(), 2);
    }

    #[test]
    fn make_plan_gives_up_after_two_garbage_answers() {
        let (fake, llm) = llm_returning(vec!["sorry".into()]);
        let e = crate::net::runtime().block_on(make_plan(&llm, "cats", M, None)).unwrap_err();
        assert!(matches!(e, PlanError::Unreadable(_)));
        assert_eq!(fake.requests().len(), 2);
    }

    #[test]
    fn an_unsafe_request_never_reaches_the_model() {
        let (fake, llm) = llm_returning(vec![GOOD.into()]);
        let e = crate::net::runtime().block_on(make_plan(&llm, "a gun", M, None)).unwrap_err();
        assert!(matches!(e, PlanError::Refused(_)));
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn make_prompts_falls_back_to_the_ideas_when_the_model_fails() {
        let (_fake, llm) = llm_returning(vec!["nonsense".into()]);
        let plan = parse_plan(GOOD, "x", M).unwrap();
        let p = crate::net::runtime().block_on(make_prompts(&llm, &plan));
        assert_eq!(p.len(), 4);
        assert!(p.iter().all(|s| s.ends_with(STYLE_SUFFIX)));
        assert!(p[0].starts_with("A kitten hanging a bauble"));
    }

    #[test]
    fn the_request_is_quoted_safely() {
        assert!(plan_request(" say \"hi\" ", M, false).starts_with("The person said: \"say 'hi'\""));
    }

    fn doll_plan() -> String {
        r#"{"safe":true,"subject":"a ginger tabby cat","picture":"a ginger tabby cat with a white chest sitting on a red sofa","setting":"living room","mood":"calm","ideas":["the ginger tabby cat chasing a butterfly","the ginger tabby cat asleep in a basket","the ginger tabby cat playing with yarn","the ginger tabby cat looking out of a window"]}"#.into()
    }

    #[test]
    fn the_plan_request_carries_the_picture_only_when_there_is_one() {
        let (fake, llm) = llm_returning(vec![doll_plan()]);
        let rt = crate::net::runtime();
        let plan = rt.block_on(make_plan(&llm, "my cat", M, Some(&[0xFF, 0xD8, 9]))).unwrap();
        rt.block_on(make_plan(&llm, "my cat", M, None)).unwrap();
        let seen = fake.requests();
        let with = &seen[0].json()["messages"][1]["content"];
        assert!(with[0]["text"].as_str().unwrap().contains("picture is attached"));
        assert!(with[1]["image_url"]["url"].as_str().unwrap().starts_with("data:image/jpeg;base64,"));
        let without = &seen[1].json()["messages"][1]["content"];
        assert!(without.is_string() && !without.as_str().unwrap().contains("picture"));
        assert_eq!(plan.picture.as_deref(), Some("a ginger tabby cat with a white chest sitting on a red sofa"));
    }

    #[test]
    fn the_plan_and_prompt_steps_are_told_the_level_and_the_picture_subject() {
        let (fake, llm) = llm_returning(vec![
            doll_plan(),
            r#"{"prompts":["a ginger tabby cat chasing a butterfly","a ginger tabby cat asleep","a ginger tabby cat with yarn","a ginger tabby cat at a window"]}"#.into(),
        ]);
        let rt = crate::net::runtime();
        let plan = rt.block_on(make_plan(&llm, "my cat", Age::Adult, Some(b"x"))).unwrap();
        assert_eq!(plan.age, Age::Adult);
        let prompts = rt.block_on(make_prompts(&llm, &plan));
        let seen = fake.requests();
        assert!(seen[0].text().contains("intricate"));
        let ask = seen[1].text();
        assert!(ask.contains("intricate") && ask.contains("ginger tabby cat with a white chest"), "{ask}");
        assert!(prompts.iter().all(|p| p.contains("ginger tabby cat") && p.contains(Age::Adult.words().as_str())));
    }

    fn everything_sent_at(age: Age) -> String {
        let (fake, llm) = llm_returning(vec![
            GOOD.into(),
            r#"{"prompts":["a fox","a owl","a deer","a hare"]}"#.into(),
        ]);
        let rt = crate::net::runtime();
        let plan = rt.block_on(make_plan(&llm, "woodland animals", age, None)).unwrap();
        let prompts = rt.block_on(make_prompts(&llm, &plan));
        let mut all: String = fake.requests().iter().map(|r| r.text()).collect::<Vec<_>>().join("\n");
        all.push_str(&prompts.join("\n"));
        all.to_lowercase()
    }

    #[test]
    fn teen_and_adult_requests_carry_no_babyish_wording() {
        for age in [Age::Teen, Age::Adult] {
            let all = everything_sent_at(age);
            for banned in ["simple", "aged 2 to 10", "for children", "cute"] {
                assert!(!all.contains(banned), "{age:?} contains {banned}");
            }
        }
    }

    #[test]
    fn a_three_year_old_gets_big_simple_shapes() {
        let all = everything_sent_at(Age::Years(3));
        assert!(all.matches("big simple shapes").count() >= 3, "{all}");
        assert!(!all.contains("aged 2 to 10") && !all.contains("for children"));
    }

    #[test]
    fn the_safety_rules_stay_in_the_system_messages_and_the_block_list_holds_at_every_age() {
        for i in 0..Age::COUNT {
            let age = Age::from_index(i);
            let system = plan_system(age).to_lowercase();
            assert!(system.contains("violence") && system.contains("nudity"), "{age:?}");
            let raw = r#"{"safe":true,"ideas":["a gun fight","a happy dog"]}"#;
            let p = parse_plan(raw, "x", age).unwrap();
            assert!(p.ideas.iter().all(|i| !i.contains("gun")));
            let p = parse_prompts(r#"{"prompts":["a gun","b","c","d"]}"#, &ideas(), age).unwrap();
            assert!(p[0].starts_with("a."));
        }
    }
}
