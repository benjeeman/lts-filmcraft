//! Builders and a seeded generator shared by the model tests. Everything is synthetic. The grid helpers are written again
//! from the spec here (`tq`, `tfloor`, `tceil`) so the tests do not lean on the code they check.

use filmcraft_time::{FrameRate, Tick};

use crate::vertical::*;
use crate::{ItemId, Label, Project, Sequence, SequenceSettings};

/// The 9:16 output of the client plan.
pub const OUT: OutputSize = OutputSize { width: 1080, height: 1920 };

/// SplitMix64: a small seeded generator, so every property test is reproducible.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / 9_007_199_254_740_992.0
    }
    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    /// Uniform in `lo..=hi`.
    pub fn int(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (hi - lo + 1).max(1) as u64;
        lo + (self.next_u64() % span) as i64
    }
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

/// 1920x1080 at 25p, the client master's raster.
pub fn hd() -> Raster {
    Raster { width: 1920, height: 1080, rate: FrameRate::FPS_25, end_frame: 100_000 }
}

/// 3840x2160 at 29.97p.
pub fn uhd() -> Raster {
    Raster { width: 3840, height: 2160, rate: FrameRate::FPS_29_97, end_frame: 100_000 }
}

/// The start tick of `frame` on `r`.
pub fn tick(r: &Raster, frame: i64) -> Tick {
    r.rate.tick_of(frame)
}

pub fn win(cx: f64, cy: f64, h: f64) -> Window {
    Window { cx, cy, h }
}

/// A whole move.
pub fn mv(frames: u32, curve: Curve) -> Arrival {
    Arrival::Move { move_frames: frames, curve, split_of: None }
}

/// A piece of a split move.
pub fn mv_split(frames: u32, curve: Curve, offset: u32, total: u32) -> Arrival {
    Arrival::Move { move_frames: frames, curve, split_of: Some(SplitOf { offset, total }) }
}

/// A `fill` position made by a person.
pub fn pos(r: &Raster, id: u32, frame: i64, window: Window, arrival: Arrival) -> Position {
    Position::new(id, tick(r, frame), window, arrival, Origin::Manual, Writer::Human)
}

/// A `fit` position, reached by a cut.
pub fn fit(r: &Raster, id: u32, frame: i64, window: Window) -> Position {
    Position { layout: Layout::Fit, ..pos(r, id, frame, window, Arrival::Cut) }
}

/// A 9:16 plan holding `positions`.
pub fn plan_of(positions: Vec<Position>) -> VerticalPlan {
    let next_position_id = positions.iter().map(|p| p.id).max().unwrap_or(0) + 1;
    VerticalPlan { positions, next_position_id, ..VerticalPlan::new(1, "9:16", OUT) }
}

/// One curve of every kind.
pub fn every_curve() -> [Curve; 9] {
    [
        Curve::Linear,
        Curve::EaseIn,
        Curve::EaseOut,
        Curve::EaseInOut,
        Curve::Speeds { start: 0.0, end: 1.75 },
        Curve::Speeds { start: 1.5, end: 0.0 },
        Curve::Bezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 },
        Curve::Ramp { r_in: 0.2, r_out: 0.2 },
        Curve::Ramp { r_in: 0.5, r_out: 0.5 },
    ]
}

/// A fixed valid plan on `hd()` with every kind of curve, zooms, a `fit` stretch, a two-piece and a three-piece split and
/// the shortest move.
pub fn mixed_plan(r: &Raster) -> VerticalPlan {
    let ramp = Curve::Ramp { r_in: 0.2, r_out: 0.2 };
    plan_of(vec![
        pos(r, 1, 10, win(600.0, 540.0, 1080.0), Arrival::Cut),
        pos(r, 2, 60, win(900.0, 540.0, 1080.0), mv(40, Curve::Linear)),
        pos(r, 3, 100, win(1100.25, 600.0, 810.0), mv(30, Curve::EaseIn)),
        pos(r, 4, 150, win(800.5, 500.0, 960.0), mv(50, Curve::EaseOut)),
        pos(r, 5, 200, win(1000.0, 540.0, 1080.0), mv(25, Curve::EaseInOut)),
        pos(r, 6, 260, win(700.0, 560.0, 1000.0), mv(60, Curve::Speeds { start: 0.0, end: 1.75 })),
        pos(r, 7, 300, win(1200.0, 540.0, 1080.0), mv(40, Curve::Speeds { start: 1.5, end: 0.0 })),
        pos(r, 8, 360, win(950.0, 520.0, 900.0), mv(45, Curve::Bezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 })),
        pos(r, 9, 400, win(640.0, 540.0, 1080.0), mv(40, Curve::Bezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 })),
        pos(r, 10, 460, win(1300.0, 600.0, 700.0), mv(60, ramp)),
        pos(r, 11, 480, win(1305.0, 610.0, 690.0), mv(20, Curve::Ramp { r_in: 0.0, r_out: 0.3 })),
        pos(r, 12, 520, win(1000.0, 540.0, 1080.0), mv(40, Curve::Ramp { r_in: 0.5, r_out: 0.5 })),
        fit(r, 13, 540, win(960.0, 540.0, 1080.0)),
        pos(r, 14, 560, win(500.0, 540.0, 1080.0), Arrival::Cut),
        pos(r, 15, 580, win(600.0, 540.0, 1080.0), mv_split(20, Curve::EaseInOut, 0, 50)),
        pos(r, 16, 610, win(1000.0, 580.0, 900.0), mv_split(30, Curve::EaseInOut, 20, 50)),
        pos(r, 17, 640, win(700.0, 540.0, 1000.0), mv_split(30, ramp, 0, 90)),
        pos(r, 18, 670, win(800.0, 520.0, 950.0), mv_split(30, ramp, 30, 90)),
        pos(r, 19, 700, win(900.0, 500.0, 900.0), mv_split(30, ramp, 60, 90)),
        pos(r, 20, 760, win(960.0, 540.0, 1080.0), mv(2, Curve::Linear)),
    ])
}

/// `q` of spec section 2.6.
pub fn tq(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// `floor_g` of spec section 2.6.
pub fn tfloor(v: f64) -> f64 {
    (v * 1000.0 + 1e-9).floor() / 1000.0
}

/// `ceil_g` of spec section 2.6.
pub fn tceil(v: f64) -> f64 {
    (v * 1000.0 - 1e-9).ceil() / 1000.0
}

/// On the grid: `q(v) == v`, bit for bit.
pub fn tgrid(v: f64) -> bool {
    v.is_finite() && tq(v).to_bits() == v.to_bits()
}

/// `(h_min, h_max)` of spec section 2.3, rounded inward onto the grid.
pub fn height_range(r: &Raster, out: OutputSize) -> (f64, f64) {
    let (w, h) = (f64::from(r.width), f64::from(r.height));
    let (ow, oh) = (f64::from(out.width), f64::from(out.height));
    (tceil(oh / 4.0), tfloor(h.min(w * oh / ow)))
}

/// The window's rectangle `[x, y, w, h]` in the operation order of spec section 2.3.
pub fn rect_of(w: &Window, out: OutputSize) -> [f64; 4] {
    let width = w.h * f64::from(out.width) / f64::from(out.height);
    [w.cx - width * 0.5, w.cy - w.h * 0.5, width, w.h]
}

/// Whether `w` lies inside `r`'s frame within `eps` and inside the height range.
pub fn inside(w: &Window, r: &Raster, out: OutputSize, eps: f64) -> bool {
    let [x, y, width, height] = rect_of(w, out);
    let (h_min, h_max) = height_range(r, out);
    let finite = [x, y, width, height].iter().all(|v| v.is_finite());
    let inside_x = x >= -eps && x + width <= f64::from(r.width) + eps;
    let inside_y = y >= -eps && y + height <= f64::from(r.height) + eps;
    finite && inside_x && inside_y && (h_min - eps..=h_max + eps).contains(&height)
}

/// Bit-for-bit equality of two windows.
pub fn same_bits(a: &Window, b: &Window) -> bool {
    a.cx.to_bits() == b.cx.to_bits() && a.cy.to_bits() == b.cy.to_bits() && a.h.to_bits() == b.h.to_bits()
}

/// A random window on the grid, inside `r`'s frame, of any legal height.
pub fn random_window(rng: &mut Rng, r: &Raster, out: OutputSize) -> Window {
    let (h_min, h_max) = height_range(r, out);
    let (fw, fh) = (f64::from(r.width), f64::from(r.height));
    let h = tq(rng.range(h_min, h_max)).max(h_min).min(h_max);
    let w = h * f64::from(out.width) / f64::from(out.height);
    let cx = tq(rng.range(w * 0.5, fw - w * 0.5)).max(tceil(w * 0.5)).min(tfloor(fw - w * 0.5));
    let cy = tq(rng.range(h * 0.5, fh - h * 0.5)).max(tceil(h * 0.5)).min(tfloor(fh - h * 0.5));
    win(cx, cy, h)
}

/// A random valid `speeds` curve: both speeds in `[0, 3]` with a sum of at most 3.
pub fn random_speeds(rng: &mut Rng) -> Curve {
    let start = rng.range(0.0, 3.0);
    Curve::Speeds { start, end: rng.range(0.0, 3.0 - start) }
}

/// A random valid ramp, the edge cases included.
pub fn random_ramp(rng: &mut Rng) -> Curve {
    match rng.int(0, 4) {
        0 => Curve::Ramp { r_in: 0.0, r_out: rng.range(0.0, 1.0) },
        1 => Curve::Ramp { r_in: rng.range(0.0, 1.0), r_out: 0.0 },
        2 => Curve::Ramp { r_in: 0.5, r_out: 0.5 },
        _ => {
            let r_in = rng.range(0.0, 0.6);
            Curve::Ramp { r_in, r_out: rng.range(0.0, 1.0 - r_in) }
        }
    }
}

/// A random valid curve of any kind. Its Bezier curves are CSS curves whose `x` handles stay inside `[0.05, 0.95]`.
pub fn random_curve(rng: &mut Rng) -> Curve {
    match rng.int(0, 7) {
        0 => Curve::Linear,
        1 => Curve::EaseIn,
        2 => Curve::EaseOut,
        3 => Curve::EaseInOut,
        4 => random_speeds(rng),
        5 => Curve::Bezier { x1: rng.range(0.05, 0.95), y1: rng.range(0.0, 1.0), x2: rng.range(0.05, 0.95), y2: rng.range(0.0, 1.0) },
        _ => random_ramp(rng),
    }
}

/// `fnv1a64:` and 16 lowercase hex digits.
pub fn digest_ok(d: &str) -> bool {
    d.len() == 24 && d.starts_with("fnv1a64:") && d.bytes().skip(8).all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A set with no plan and no data.
pub fn empty_set() -> VerticalSet {
    VerticalSet {
        plans: Vec::new(),
        walls: WallSet { use_edit_points: true, stored: Vec::new(), suppressed: Vec::new(), candidates: Vec::new(), analysed: Vec::new() },
        parts: Vec::new(),
        regions: Vec::new(),
        annotations: Vec::new(),
        box_tracks: Vec::new(),
        active_area: Vec::new(),
        agent_overwrite: None,
        part_audio: PartAudio::default(),
        delivery: DeliveryInfo::default(),
        reference: None,
        deliveries: Vec::new(),
        next_plan_id: 1,
        next_part_id: 1,
        next_region_id: 1,
        next_annotation_id: 1,
    }
}

/// A delivery record of `kind` with the given conform state.
pub fn delivery(kind: DeliveryKind, part: Option<u32>, conform: ConformState) -> DeliveryRecord {
    DeliveryRecord {
        plan: 1,
        part,
        kind,
        profile: "review-720".into(),
        profile_digest: "fnv1a64:00000000000000aa".into(),
        version: 2,
        framing_digest: "fnv1a64:0123456789abcdef".into(),
        output_digest: "fnv1a64:fedcba9876543210".into(),
        file: "D:/deliveries/S01E12_P07_v2.mp4".into(),
        bytes: 1_234_567,
        sha256: "5f".repeat(32),
        frames: part.map(|_| 281),
        loudness: None,
        source: SourceRecord { media: Vec::new(), conform },
        by: "agent".into(),
        when: 1_790_000_400,
        run_id: None,
    }
}

/// A set that uses every stored type and every optional field (synthetic data only).
pub fn full_set(r: &Raster) -> VerticalSet {
    let ramp = Curve::Ramp { r_in: 0.2, r_out: 0.2 };
    let opener = Position {
        caption_anchor: Some(CaptionAnchor::Top),
        label: "Dana".into(),
        color: Some(Label::Rose),
        tags: vec!["speaker:Dana".into()],
        note: "active speaker".into(),
        subjects: vec!["char:dana".into()],
        focus: Some([812.4, 400.25]),
        confidence: Some(0.875),
        risk: "check the eyeline".into(),
        origin: Origin::Agent,
        modified_by: Writer::Agent,
        ..pos(r, 31, 4805, win(812.4, 540.0, 1080.0), Arrival::Cut)
    };
    let proposal = Proposal { at: tick(r, 4935), window: win(960.0, 540.0, 1080.0), layout: Layout::Fill, arrival: mv(50, Curve::EaseInOut) };
    let walk = Position {
        reviewed: true,
        locked: true,
        origin: Origin::Agent,
        flags: vec![Flag::PictureChanged, Flag::MoveShortened, Flag::TimeApproximate],
        proposed: Some(proposal),
        ..pos(r, 32, 4935, win(948.2, 540.0, 1080.0), mv(40, Curve::EaseInOut))
    };
    let mut first = VerticalPlan::new(1, "9:16", OUT);
    first.filter = Filter::Bilinear;
    first.starting_window = Some(win(960.0, 540.0, 1080.0));
    first.captions = VerticalCaptions {
        area: Some([120, 288, 768, 960]),
        size: Some(54.0),
        min_size: Some(40.0),
        max_lines: 3,
        keep_line_breaks: false,
        anchor: CaptionAnchor::Top,
        track: Some(2),
        edge: CueEdge::Drop,
        min_clipped_frames: 5,
    };
    first.fit_background = FitBackground::Black;
    first.positions = vec![
        opener,
        walk.clone(),
        pos(r, 33, 4960, win(1000.5, 500.25, 810.0), mv_split(20, Curve::Speeds { start: 0.0, end: 1.75 }, 0, 50)),
        pos(r, 34, 4990, win(1100.0, 520.0, 900.0), mv_split(30, Curve::Speeds { start: 0.0, end: 1.75 }, 20, 50)),
        pos(r, 35, 5100, win(900.0, 540.0, 1080.0), mv(60, Curve::Bezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 })),
        pos(r, 36, 5200, win(1000.5, 500.25, 810.0), mv(50, ramp)),
        Position { caption_anchor: Some(CaptionAnchor::Bottom), origin: Origin::Seeded, ..fit(r, 37, 5300, win(960.0, 540.0, 1080.0)) },
        Position { origin: Origin::Reanchored, ..pos(r, 38, 5400, win(700.0, 540.0, 1080.0), Arrival::Cut) },
    ];
    first.part_approvals = vec![PartApproval {
        part: 7,
        framing_digest: "fnv1a64:0123456789abcdef".into(),
        output_digest: "fnv1a64:fedcba9876543210".into(),
        by: "Dana Editor".into(),
        when: 1_790_000_000,
        note: "approved for the first cut".into(),
    }];
    first.attention = vec![AttentionMark { at: tick(r, 4900), by: "Dana Editor".into(), when: 1_790_000_100, note: "eyeline".into() }];
    first.overwritten = vec![OverwrittenRange {
        id: 1,
        from: tick(r, 4895),
        to: tick(r, 4935),
        positions: vec![walk],
        label: "vertical.replaceRange".into(),
        run_id: Some("run-17".into()),
        when: 1_790_000_200,
    }];
    first.next_position_id = 39;
    first.next_record_id = 2;
    let mut second = VerticalPlan::new(2, "4:5", OutputSize { width: 1080, height: 1350 });
    second.positions = vec![pos(r, 1, 0, win(960.0, 540.0, 1080.0), Arrival::Cut)];
    second.next_position_id = 2;
    let walls = WallSet {
        use_edit_points: true,
        stored: vec![
            StoredWall { at: tick(r, 4700), origin: WallOrigin::Agent, score: None },
            StoredWall { at: tick(r, 4805), origin: WallOrigin::Detected, score: Some(0.87) },
            StoredWall { at: tick(r, 5300), origin: WallOrigin::Manual, score: None },
            StoredWall { at: tick(r, 5400), origin: WallOrigin::Marker, score: None },
            StoredWall { at: tick(r, 5500), origin: WallOrigin::Reanchored, score: None },
        ],
        suppressed: vec![tick(r, 4750)],
        candidates: vec![Candidate { at: tick(r, 4900), score: 0.19 }],
        analysed: vec![FrameRange { from: tick(r, 0), to: tick(r, 32_294) }],
    };
    let parts = vec![
        Part {
            id: 7,
            name: "P07".into(),
            from: tick(r, 4703),
            to: tick(r, 4984),
            audio: Some(PartAudio { fade_in_frames: 3, fade_out_frames: 6 }),
            meta: PartMeta { title: "Dana walks".into(), description: "שלום, a Hebrew line".into() },
        },
        Part { id: 8, name: "P08".into(), from: tick(r, 4984), to: tick(r, 5600), audio: None, meta: PartMeta::default() },
    ];
    let regions = vec![region(r, 1, "logo", RegionMode::KeepIn), region(r, 2, "super", RegionMode::AllOrNothing), region(r, 3, "crew", RegionMode::KeepOut)];
    let speech = Annotation {
        id: 1,
        kind: "speech".into(),
        from: tick(r, 4810),
        to: tick(r, 4890),
        label: "char:dana".into(),
        score: Some(0.875),
        origin: Origin::Agent,
        modified_by: Writer::Agent,
    };
    let music = Annotation { id: 2, kind: "music".into(), label: String::new(), score: None, ..speech.clone() };
    let keys = vec![
        KeyBox { at: tick(r, 4805), rect: [700.0, 300.0, 200.0, 240.0], facing: Some(Facing::Left), gone: false },
        KeyBox { at: tick(r, 4850), rect: [720.5, 310.0, 200.0, 240.0], facing: Some(Facing::Camera), gone: true },
        KeyBox { at: tick(r, 4900), rect: [740.0, 320.0, 200.0, 240.0], facing: None, gone: false },
    ];
    let dana = BoxTrack { id: "char:dana".into(), label: "דנה".into(), kind: "face".into(), keys, origin: Origin::Agent, modified_by: Writer::Agent };
    let area = ActiveArea { from: tick(r, 0), to: tick(r, 1000), rect: [0.0, 132.0, 1920.0, 816.0], origin: Origin::Manual, modified_by: Writer::Human };
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
    let source = SourceRecord {
        media: vec![SourceMedia { item: ItemId(3), path: "D:/media/S01E12.mxf".into(), bytes: 9_876_543_210, sha256: "ab".repeat(32) }],
        conform: ConformState::Match,
    };
    let run_id = Some("run-17".to_string());
    let video = DeliveryRecord { loudness: Some(loudness), source, run_id, ..delivery(DeliveryKind::Video, Some(7), ConformState::Match) };
    VerticalSet {
        plans: vec![first, second],
        walls,
        parts,
        regions,
        annotations: vec![speech, music],
        box_tracks: vec![dana],
        active_area: vec![area],
        agent_overwrite: Some(OverwriteSetting { allowed: true, by: "Dana Editor".into(), when: 1_790_000_050, sequence: ItemId(7) }),
        part_audio: PartAudio { fade_in_frames: 3, fade_out_frames: 5 },
        delivery: DeliveryInfo { series: "Series".into(), season: "S01".into(), episode: "E12".into() },
        reference: Some(reference),
        deliveries: vec![
            video,
            delivery(DeliveryKind::Sidecar, Some(7), ConformState::Unverified),
            delivery(DeliveryKind::Cover, Some(7), ConformState::NoReference),
            delivery(DeliveryKind::Manifest, None, ConformState::Match),
            delivery(DeliveryKind::BreakList, None, ConformState::Match),
        ],
        next_plan_id: 3,
        next_part_id: 9,
        next_region_id: 4,
        next_annotation_id: 3,
    }
}

/// A protected region over frames 4700-5000.
pub fn region(r: &Raster, id: u32, label: &str, mode: RegionMode) -> ProtectedRegion {
    ProtectedRegion {
        id,
        label: label.into(),
        from: tick(r, 4700),
        to: tick(r, 5000),
        rect: [1700.0, 40.0, 180.0, 90.0],
        mode,
        origin: Origin::Manual,
        modified_by: Writer::Human,
    }
}

/// A 25p 1920x1080 sequence holding `set`.
pub fn sequence_with(set: VerticalSet) -> Sequence {
    let mut project = Project::new("Vertical Edit tests");
    let settings = SequenceSettings { frame_rate: FrameRate::FPS_25, ..SequenceSettings::default() };
    let id = project.new_sequence("Episode 12", settings, 1, 1, None);
    let mut seq = project.sequence(id).cloned().expect("the sequence was just made");
    seq.vertical = Some(Box::new(set));
    seq
}
