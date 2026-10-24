//! T-M8b: a project with a full vertical set round-trips through `encode_with_view` and `decode` with identical bits and
//! digests, and a v12 file without vertical data decodes unchanged (spec sections 2.6 and 2.8). Synthetic data only.

use filmcraft_format::{SCHEMA_VERSION, decode, encode, encode_with_view};
use filmcraft_project::vertical::*;
use filmcraft_project::{ItemId, Label, Project, ProjectView, SequenceSettings};
use filmcraft_time::{FrameRate, Tick};
use serde_json::Value;

const OUT: OutputSize = OutputSize { width: 1080, height: 1920 };

fn tick(frame: i64) -> Tick {
    FrameRate::FPS_25.tick_of(frame)
}

fn win(cx: f64, cy: f64, h: f64) -> Window {
    Window { cx, cy, h }
}

fn mv(frames: u32, curve: Curve, split_of: Option<SplitOf>) -> Arrival {
    Arrival::Move { move_frames: frames, curve, split_of }
}

fn pos(id: u32, frame: i64, window: Window, arrival: Arrival) -> Position {
    Position::new(id, tick(frame), window, arrival, Origin::Manual, Writer::Human)
}

/// A vertical set that uses every stored type and every optional field.
fn full_set() -> VerticalSet {
    let opener = Position {
        caption_anchor: Some(CaptionAnchor::Top),
        label: "Dana CU".into(),
        color: Some(Label::Rose),
        tags: vec!["speaker:Dana".into()],
        note: "active speaker".into(),
        subjects: vec!["char:dana".into()],
        focus: Some([812.4, 400.25]),
        confidence: Some(0.875),
        risk: "check the eyeline".into(),
        origin: Origin::Agent,
        modified_by: Writer::Agent,
        ..pos(31, 4805, win(812.4, 540.0, 1080.0), Arrival::Cut)
    };
    let proposal = Proposal { at: tick(4935), window: win(960.0, 540.0, 1080.0), layout: Layout::Fill, arrival: mv(50, Curve::EaseInOut, None) };
    let walk = Position {
        reviewed: true,
        locked: true,
        flags: vec![Flag::PictureChanged, Flag::MoveShortened, Flag::TimeApproximate],
        proposed: Some(proposal),
        ..pos(32, 4935, win(948.2, 540.0, 1080.0), mv(40, Curve::EaseInOut, None))
    };
    // values that a lossy float parser would double-round
    let speeds = Curve::Speeds { start: 0.1 + 0.2, end: 1.0 / 3.0 * 5.0 };
    let bezier = Curve::Bezier { x1: 1.0 / 3.0, y1: 0.1 + 0.7, x2: 2.0 / 3.0, y2: 0.9999999999999999 };
    let ramp = Curve::Ramp { r_in: 0.2 + 0.1, r_out: 0.7 / 3.0 };
    let mut first = VerticalPlan::new(1, "9:16", OUT);
    first.filter = Filter::Bilinear;
    first.starting_window = Some(win(960.0, 540.0, 1080.0));
    first.captions = VerticalCaptions { area: Some([120, 288, 768, 960]), max_lines: 3, track: Some(2), edge: CueEdge::Drop, ..VerticalCaptions::default() };
    first.fit_background = FitBackground::Black;
    first.positions = vec![
        opener,
        walk.clone(),
        pos(33, 4960, win(1000.5, 500.25, 810.0), mv(20, speeds, Some(SplitOf { offset: 0, total: 50 }))),
        pos(34, 4990, win(1100.001, 520.0, 900.0), mv(30, speeds, Some(SplitOf { offset: 20, total: 50 }))),
        pos(35, 5100, win(900.0, 540.0, 1080.0), mv(60, bezier, None)),
        pos(36, 5200, win(1000.5, 500.25, 810.0), mv(50, ramp, None)),
        Position { layout: Layout::Fit, origin: Origin::Seeded, ..pos(37, 5300, win(960.0, 540.0, 1080.0), Arrival::Cut) },
        Position { origin: Origin::Reanchored, ..pos(38, 5400, win(700.0, 540.0, 1080.0), Arrival::Cut) },
    ];
    first.part_approvals = vec![PartApproval {
        part: 7,
        framing_digest: "fnv1a64:0123456789abcdef".into(),
        output_digest: "fnv1a64:fedcba9876543210".into(),
        by: "Dana Editor".into(),
        when: 1_790_000_000,
        note: "approved".into(),
    }];
    first.attention = vec![AttentionMark { at: tick(4900), by: "Dana Editor".into(), when: 1_790_000_100, note: "eyeline".into() }];
    first.overwritten = vec![OverwrittenRange {
        id: 1,
        from: tick(4895),
        to: tick(4935),
        positions: vec![walk],
        label: "vertical.replaceRange".into(),
        run_id: Some("run-17".into()),
        when: 1_790_000_200,
    }];
    first.next_position_id = 39;
    first.next_record_id = 2;
    let stored = vec![
        StoredWall { at: tick(4700), origin: WallOrigin::Agent, score: None },
        StoredWall { at: tick(4805), origin: WallOrigin::Detected, score: Some(0.87) },
        StoredWall { at: tick(5500), origin: WallOrigin::Reanchored, score: None },
    ];
    let walls = WallSet {
        use_edit_points: true,
        stored,
        suppressed: vec![tick(4750)],
        candidates: vec![Candidate { at: tick(4900), score: 0.19 }],
        analysed: vec![FrameRange { from: tick(0), to: tick(32_294) }],
    };
    let meta = PartMeta { title: "Dana walks".into(), description: "שלום, a Hebrew line".into() };
    let part = Part { id: 7, name: "P07".into(), from: tick(4703), to: tick(4985), audio: Some(PartAudio { fade_in_frames: 3, fade_out_frames: 6 }), meta };
    let region = ProtectedRegion {
        id: 1,
        label: "logo".into(),
        from: tick(4700),
        to: tick(5000),
        rect: [1700.0, 40.0, 180.0, 90.0],
        mode: RegionMode::KeepOut,
        origin: Origin::Manual,
        modified_by: Writer::Human,
    };
    let speech = Annotation {
        id: 1,
        kind: "speech".into(),
        from: tick(4810),
        to: tick(4890),
        label: "char:dana".into(),
        score: Some(0.875),
        origin: Origin::Agent,
        modified_by: Writer::Agent,
    };
    let keys = vec![
        KeyBox { at: tick(4805), rect: [700.0, 300.0, 200.0, 240.0], facing: Some(Facing::Left), gone: false },
        KeyBox { at: tick(4850), rect: [720.5, 310.0, 200.0, 240.0], facing: None, gone: true },
    ];
    let dana = BoxTrack { id: "char:dana".into(), label: "דנה".into(), kind: "face".into(), keys, origin: Origin::Agent, modified_by: Writer::Agent };
    let area = ActiveArea { from: tick(0), to: tick(1000), rect: [0.0, 132.0, 1920.0, 816.0], origin: Origin::Manual, modified_by: Writer::Human };
    let signature = MediaSignature {
        item: ItemId(3),
        path: "D:/media/S01E12.mxf".into(),
        frames: 32_294,
        rate: FrameRate::FPS_25,
        start_timecode: 90_000,
        width: 1920,
        height: 1080,
    };
    let grids = vec!["AAAA".into(), "BBBB".into()];
    let reference = ConformReference { when: 1_790_000_300, media: vec![signature], frames: vec![4804, 4805], grids, wall_pairs: vec![4805] };
    let loudness = LoudnessRecord {
        measured_lufs: -21.5,
        gain_db: 7.5,
        integrated_lufs: -14.0,
        true_peak_dbtp: -1.25,
        max_short_term_lufs: -11.5,
        lra_lu: 6.5,
        correlation: 0.875,
        min_correlation: 0.25,
    };
    let media = vec![SourceMedia { item: ItemId(3), path: "D:/media/S01E12.mxf".into(), bytes: 9_876_543_210, sha256: "ab".repeat(32) }];
    let record = DeliveryRecord {
        plan: 1,
        part: Some(7),
        kind: DeliveryKind::Video,
        profile: "review-720".into(),
        profile_digest: "fnv1a64:00000000000000aa".into(),
        version: 2,
        framing_digest: "fnv1a64:0123456789abcdef".into(),
        output_digest: "fnv1a64:fedcba9876543210".into(),
        file: "D:/deliveries/S01E12_P07_v2.mp4".into(),
        bytes: 1_234_567,
        sha256: "5f".repeat(32),
        frames: Some(281),
        loudness: Some(loudness),
        source: SourceRecord { media, conform: ConformState::Unverified },
        by: "agent".into(),
        when: 1_790_000_400,
        run_id: Some("run-17".into()),
    };
    let mut second = VerticalPlan::new(2, "4:5", OutputSize { width: 1080, height: 1350 });
    second.positions = vec![pos(1, 0, win(960.0, 540.0, 1080.0), Arrival::Cut)];
    second.next_position_id = 2;
    VerticalSet {
        plans: vec![first, second],
        walls,
        parts: vec![part],
        regions: vec![region],
        annotations: vec![speech],
        box_tracks: vec![dana],
        active_area: vec![area],
        agent_overwrite: Some(OverwriteSetting { allowed: true, by: "Dana Editor".into(), when: 1_790_000_050, sequence: ItemId(7) }),
        part_audio: PartAudio { fade_in_frames: 3, fade_out_frames: 5 },
        delivery: DeliveryInfo { series: "Series".into(), season: "S01".into(), episode: "E12".into() },
        reference: Some(reference),
        deliveries: vec![record],
        next_plan_id: 3,
        next_part_id: 8,
        next_region_id: 2,
        next_annotation_id: 2,
    }
}

/// A project with a 25p sequence that holds `set`, and the sequence's id.
fn project_with(set: Option<VerticalSet>) -> (Project, ItemId) {
    let mut project = Project::new("Vertical Edit round trip");
    let settings = SequenceSettings { frame_rate: FrameRate::FPS_25, ..SequenceSettings::default() };
    let id = project.new_sequence("Episode 12", settings, 1, 1, None);
    if let Some(seq) = project.sequence_mut(id) {
        seq.vertical = set.map(Box::new);
    }
    (project, id)
}

/// The three digests of every plan of the sequence's set.
fn digests(project: &Project, id: ItemId) -> Vec<String> {
    let seq = project.sequence(id).expect("the sequence");
    let set = seq.vertical.as_deref().expect("the set");
    let mut out: Vec<String> = set.plans.iter().map(|plan| plan_digest(seq, plan)).collect();
    out.push(data_digest(set));
    out.push(walls_digest(set));
    out
}

#[test]
fn t_m8b_schema_13_is_the_vertical_edit_schema() {
    assert_eq!(SCHEMA_VERSION, 13);
}

#[test]
fn t_m8b_a_full_vertical_set_round_trips_bit_for_bit() {
    let (project, id) = project_with(Some(full_set()));
    let view = ProjectView { open_sequences: vec![id], active_sequence: Some(id), sequences: Default::default() };
    let before = digests(&project, id);
    assert!(before.iter().all(|d| d.len() == 24 && d.starts_with("fnv1a64:")), "{before:?}");
    for pretty in [true, false] {
        let bytes = encode_with_view(&project, Some(&view), pretty);
        let loaded = decode(&bytes).unwrap_or_else(|e| panic!("the file does not open: {e}"));
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert!(!loaded.migrated());
        assert_eq!(loaded.project, project);
        assert_eq!(loaded.view.as_ref(), Some(&view));
        // identical bits: the project writes back to the same bytes, and every digest is unchanged
        assert_eq!(encode_with_view(&loaded.project, Some(&view), pretty), bytes);
        assert_eq!(digests(&loaded.project, id), before);
        let text = String::from_utf8(bytes).expect("UTF-8");
        assert!(text.contains("\"vertical\"") && text.contains("\"moveFrames\"") && text.contains("\"splitOf\""));
    }
}

#[test]
fn t_m8b_a_v12_file_without_vertical_data_decodes_unchanged() {
    let (project, id) = project_with(None);
    let mut doc: Value = serde_json::from_slice(&encode(&project, false)).expect("JSON");
    doc["schema_version"] = Value::from(12);
    let loaded = decode(&serde_json::to_vec(&doc).expect("JSON")).unwrap_or_else(|e| panic!("a v12 file does not open: {e}"));
    assert_eq!(loaded.schema_version, 12);
    assert!(loaded.migrated());
    assert_eq!(loaded.project, project);
    assert!(loaded.project.sequence(id).is_some_and(|seq| seq.vertical.is_none()));
    // a sequence without a set writes no `vertical` key
    let text = String::from_utf8(encode(&project, true)).expect("UTF-8");
    assert!(!text.contains("\"vertical\""));
}
