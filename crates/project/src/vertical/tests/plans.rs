//! `vertical.create`, `vertical.update` and `vertical.delete` (spec section 7.3): presets and sizes, default names, the
//! set made on the first plan, ids from the set's counter, `plan_exists` for a retried create, at most 16 plans, settings
//! (filter, fit background, the starting window, captions), and the plan digest as `expect`.

use serde_json::json;

use super::fixtures::*;
use super::support::win;
use crate::vertical::*;

fn create(name: Option<&str>, preset: Option<Preset>, size: Option<(u32, u32)>) -> Write {
    Write::Create(CreatePlan { name: name.map(String::from), preset, width: size.map(|s| s.0), height: size.map(|s| s.1), filter: None })
}

#[test]
fn the_first_plan_makes_the_set_with_the_defaults() {
    let mut fx = Fx::new(1_000);
    let planned = fx.apply(create(None, None, None));
    assert_eq!((planned.plan, planned.ids.clone(), planned.label.as_str()), (Some(1), vec![1], "Vertical: new plan 9:16"));
    let set = fx.set();
    assert!(set.walls.use_edit_points, "edit points are walls by default");
    assert!(set.walls.stored.is_empty() && set.parts.is_empty());
    assert_eq!(set.next_plan_id, 2);
    let plan = fx.plan(1);
    let want = ("9:16", OutputSize { width: 1080, height: 1920 }, Filter::CatmullRom, FitBackground::Blur);
    assert_eq!((plan.name.as_str(), plan.output, plan.filter, plan.fit_background), want);
    assert_eq!((plan.captions.clone(), plan.starting_window, plan.positions.len(), plan.next_position_id), (VerticalCaptions::default(), None, 0, 1));
    let e = fx.refuse(create(None, None, None));
    assert_eq!((e.code, e.detail["id"].clone()), ("plan_exists", json!(1)), "a retried create does not duplicate a plan");
}

#[test]
fn presets_sizes_names_and_filters() {
    let mut fx = Fx::new(1_000);
    fx.apply(create(None, Some(Preset::FourFive), None));
    fx.apply(create(None, Some(Preset::OneOne), None));
    fx.apply(create(Some("vertical 720"), None, Some((720, 1280))));
    let planned = fx.apply(Write::Create(CreatePlan { filter: Some(Filter::Bilinear), ..Default::default() }));
    let plans: Vec<(u32, String, u32, u32)> = fx.set().plans.iter().map(|p| (p.id, p.name.clone(), p.output.width, p.output.height)).collect();
    assert_eq!(
        plans,
        vec![(1, "4:5".into(), 1080, 1350), (2, "1:1".into(), 1080, 1080), (3, "vertical 720".into(), 720, 1280), (4, "9:16".into(), 1080, 1920)]
    );
    assert_eq!(fx.plan(4).filter, Filter::Bilinear);
    assert_eq!(planned.plan, Some(4));
    let e = fx.refuse(create(None, None, Some((2160, 3840))));
    assert_eq!((e.code, e.detail["id"].clone()), ("plan_exists", json!(4)), "a custom size is named by its ratio");
    let e = fx.refuse(create(None, None, Some((1080, 0))));
    assert_eq!(e.code, "invalid_output_size");
    let half = Write::Create(CreatePlan { width: Some(1080), ..Default::default() });
    assert_eq!(fx.refuse(half).code, "invalid_output_size", "a size needs both sides");
}

#[test]
fn at_most_16_plans_and_ids_are_never_reused() {
    let mut fx = Fx::new(1_000);
    for n in 0..16 {
        fx.apply(create(Some(&format!("plan {n}")), None, None));
    }
    let e = fx.refuse(create(Some("plan 16"), None, None));
    assert_eq!((e.code, e.detail["key"].clone(), e.detail["bound"].clone()), ("too_many", json!("plans"), json!(16)));
    let planned = fx.apply(Write::Delete(DeletePlan { plan: 3, expect: None }));
    assert_eq!(planned.label, "Vertical: delete plan plan 2");
    assert_eq!(fx.set().plans.len(), 15);
    assert_eq!(fx.apply(create(Some("again"), None, None)).plan, Some(17), "the counter never goes back");
}

#[test]
fn update_changes_settings_and_keeps_names_unique() {
    let mut fx = Fx::new(1_000);
    fx.apply(create(None, None, None));
    fx.apply(create(None, Some(Preset::FourFive), None));
    let update = Write::Update;
    let e = fx.refuse(update(UpdatePlan { plan: 2, name: Some("9:16".into()), ..Default::default() }));
    assert_eq!((e.code, e.detail["id"].clone()), ("plan_exists", json!(1)));
    let shorts = UpdatePlan { plan: 1, name: Some("Shorts".into()), filter: Some(Filter::Bilinear), ..Default::default() };
    fx.apply(update(UpdatePlan { fit_background: Some(FitBackground::Black), ..shorts }));
    let plan = fx.plan(1);
    assert_eq!((plan.name.as_str(), plan.filter, plan.fit_background), ("Shorts", Filter::Bilinear, FitBackground::Black));
    fx.apply(update(UpdatePlan { plan: 1, starting_window: Some(Some(win(800.0, 540.0, 1080.0))), ..Default::default() }));
    assert_eq!(fx.plan(1).starting_window, Some(win(800.0, 540.0, 1080.0)));
    fx.apply(update(UpdatePlan { plan: 1, starting_window: Some(None), ..Default::default() }));
    assert_eq!(fx.plan(1).starting_window, None, "null clears it");
    let outside = UpdatePlan { plan: 1, starting_window: Some(Some(win(2_000.0, 540.0, 1080.0))), ..Default::default() };
    assert_eq!(fx.refuse(update(outside.clone())).code, "outside_picture");
    fx.apply(update(UpdatePlan { clamp: true, ..outside }));
    assert_eq!(fx.plan(1).starting_window, Some(win(1616.25, 540.0, 1080.0)));
    let captions = VerticalCaptions { max_lines: 5, ..Default::default() };
    let e = fx.refuse(update(UpdatePlan { plan: 1, captions: Some(captions), ..Default::default() }));
    assert_eq!((e.code, e.detail["key"].clone()), ("out_of_range", json!("maxLines")), "1 to 4 lines");
    assert!(!accepted(fx.write(update(UpdatePlan { plan: 1, name: Some("Shorts".into()), ..Default::default() }))).changed, "a restatement");
}

#[test]
fn expect_is_the_plan_digest() {
    let mut fx = Fx::new(1_000);
    fx.apply(create(None, None, None));
    let digest = plan_digest(fx.seq(), fx.plan(1));
    let stale = Write::Delete(DeletePlan { plan: 1, expect: Some("fnv1a64:0000000000000000".into()) });
    let e = fx.refuse(stale);
    assert_eq!((e.code, e.detail["digest"].clone()), ("stale_plan", json!(digest)));
    fx.apply(Write::Delete(DeletePlan { plan: 1, expect: Some(digest) }));
    assert!(fx.set().plans.is_empty(), "the set stays, with no plan");
    assert_eq!(fx.refuse(Write::Delete(DeletePlan { plan: 1, expect: None })).code, "not_found");
}
