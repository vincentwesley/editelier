use editelier_core::*;

const FPS: FrameRate = FrameRate::FPS_29_97;

fn secs(s: i64) -> Tick {
    Tick(s * TICKS_PER_SECOND)
}

fn frames(n: i64) -> Tick {
    FPS.tick_of(n)
}

fn video(duration: Tick, vfr: bool) -> MediaMeta {
    MediaMeta {
        kind: MediaKind::Video,
        duration,
        fps: Some(FPS),
        is_vfr: vfr,
        has_audio: true,
    }
}

/// Session with a 29.97 project, two 60 s videos (ids 1, 2), one 120 s audio file (id 3).
fn session() -> Session {
    let mut s = Session::new(Project::new(Settings {
        fps: FPS,
        ..Settings::default()
    }));
    for (path, meta) in [
        ("a.mp4", video(secs(60), false)),
        ("b.mp4", video(secs(60), true)),
        (
            "m.wav",
            MediaMeta {
                kind: MediaKind::Audio,
                duration: secs(120),
                fps: None,
                is_vfr: false,
                has_audio: true,
            },
        ),
    ] {
        s.execute(&Command::AddMedia {
            path: path.into(),
            meta,
        })
        .unwrap();
    }
    s
}

fn insert(s: &mut Session, media: Id, a: i64, b: i64) -> Id {
    s.execute(&Command::InsertClip {
        media,
        source_in: frames(a),
        source_out: frames(b),
        index: None,
    })
    .unwrap()
    .unwrap()
}

/// xorshift64: deterministic "random" for property-ish tests without a dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

#[test]
fn frame_math_is_exact_at_ntsc_rates() {
    assert_eq!(FrameRate::FPS_29_97.frame_duration(), Tick(8_475_667_200));
    assert_eq!(FrameRate::FPS_59_94.frame_duration(), Tick(4_237_833_600));
    for rate in [
        FrameRate::FPS_29_97,
        FrameRate::FPS_59_94,
        FrameRate::FPS_23_976,
    ] {
        // 30000 frames at 29.97 is exactly 1001 s; likewise for the other NTSC rates.
        assert_eq!(rate.tick_of(rate.num), secs(rate.den));
        for n in [0, 1, 29, 30, 1799, 1800, 17982, 107_892, 1_000_000] {
            let t = rate.tick_of(n);
            assert_eq!(
                t,
                rate.frame_duration() * n,
                "frames are an exact multiple at {rate}"
            );
            assert_eq!(rate.frame_at(t), n);
            assert_eq!(rate.frame_at(t - Tick(1)), n - 1);
            assert_eq!(rate.snap(t + rate.frame_duration() - Tick(1)), t);
            assert_eq!(
                rate.snap_nearest(t + rate.frame_duration() / 2 - Tick(1)),
                t
            );
        }
    }
}

#[test]
fn trims_snap_to_frames_and_clamp_to_media() {
    let mut s = session();
    let id = insert(&mut s, 1, 0, 300);
    s.execute(&Command::TrimEnd {
        id,
        source_out: frames(100) + Tick(5),
        drag: None,
    })
    .unwrap();
    assert_eq!(s.project().timeline.main[0].source_out, frames(100));
    s.execute(&Command::TrimEnd {
        id,
        source_out: secs(500),
        drag: None,
    })
    .unwrap();
    assert_eq!(s.project().timeline.main[0].source_out, FPS.snap(secs(60)));
    s.execute(&Command::TrimStart {
        id,
        source_in: secs(-5),
        drag: None,
    })
    .unwrap();
    assert_eq!(s.project().timeline.main[0].source_in, Tick::ZERO);
    s.execute(&Command::TrimStart {
        id,
        source_in: secs(1000),
        drag: None,
    })
    .unwrap();
    assert_eq!(
        s.project().timeline.main[0].duration(),
        FPS.frame_duration()
    );
    // Trim out then back in restores the clip exactly.
    let before = s.project().timeline.main.clone();
    s.execute(&Command::TrimEnd {
        id,
        source_out: frames(10),
        drag: None,
    })
    .unwrap();
    s.execute(&Command::TrimEnd {
        id,
        source_out: before[0].source_out,
        drag: None,
    })
    .unwrap();
    assert_eq!(s.project().timeline.main, before);
}

#[test]
fn random_splits_then_joins_round_trip() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..50 {
        let mut s = session();
        insert(&mut s, 1, 0, 900);
        insert(&mut s, 2, 30, 600);
        let original = s.project().timeline.main.clone();
        let total = s.project().duration();
        for _ in 0..20 {
            let at = Tick(rng.next(total.0 as u64) as i64);
            let _ = s.execute(&Command::Split { at }); // a split on an existing cut is refused
        }
        assert_eq!(
            s.project().duration(),
            total,
            "splitting never changes duration"
        );
        let mut i = 0;
        while i + 1 < s.project().timeline.main.len() {
            let id = s.project().timeline.main[i].id;
            if s.execute(&Command::Join { id }).is_err() {
                i += 1;
            }
        }
        assert_eq!(s.project().timeline.main, original);
    }
}

#[test]
fn join_refuses_non_contiguous_clips() {
    let mut s = session();
    let a = insert(&mut s, 1, 0, 100);
    insert(&mut s, 1, 101, 200);
    assert!(s.execute(&Command::Join { id: a }).is_err());
}

#[test]
fn ripple_delete_closes_the_gap() {
    let mut s = session();
    insert(&mut s, 1, 0, 100);
    let mid = insert(&mut s, 2, 0, 45);
    let last = insert(&mut s, 1, 200, 260);
    s.execute(&Command::RippleDelete { id: mid }).unwrap();
    let p = s.project();
    let (_, start, c) = p.clip_at(frames(100)).unwrap();
    assert_eq!((start, c.id), (frames(100), last));
    assert_eq!(p.duration(), frames(160));
}

#[test]
fn undo_all_and_redo_all_are_byte_exact() {
    let mut s = Session::new((**session().project()).clone()); // media in, empty history
    let initial = to_json(s.project());
    let mut rng = Rng(42);
    let mut applied = 0;
    for step in 0..500 {
        let p = s.project().clone();
        let main_id = |r: &mut Rng| {
            p.timeline
                .main
                .get(r.next(p.timeline.main.len() as u64) as usize)
                .map_or(999, |c| c.id)
        };
        let title_id = |r: &mut Rng| {
            p.timeline
                .titles
                .get(r.next(p.timeline.titles.len() as u64) as usize)
                .map_or(999, |c| c.id)
        };
        let music_id = |r: &mut Rng| {
            p.timeline
                .music
                .get(r.next(p.timeline.music.len() as u64) as usize)
                .map_or(999, |c| c.id)
        };
        let t = |r: &mut Rng| frames(r.next(2000) as i64);
        let cmd = match rng.next(12) {
            0 | 1 => Command::InsertClip {
                media: 1 + rng.next(2),
                source_in: t(&mut rng),
                source_out: t(&mut rng),
                index: None,
            },
            2 => Command::TrimStart {
                id: main_id(&mut rng),
                source_in: t(&mut rng),
                drag: Some(step / 3),
            },
            3 => Command::TrimEnd {
                id: main_id(&mut rng),
                source_out: t(&mut rng),
                drag: None,
            },
            4 => Command::Split {
                at: Tick(rng.next(p.duration().0.max(1) as u64) as i64),
            },
            5 => Command::Join {
                id: main_id(&mut rng),
            },
            6 => Command::RippleDelete {
                id: main_id(&mut rng),
            },
            7 => Command::MoveClip {
                id: main_id(&mut rng),
                index: rng.next(4) as usize,
            },
            8 => Command::AddTitle {
                start: t(&mut rng),
                duration: t(&mut rng),
                text: format!("t{step}"),
                style: None,
                position: None,
            },
            9 => Command::EditTitle {
                id: title_id(&mut rng),
                start: None,
                duration: None,
                text: Some("edited".into()),
                style: None,
                position: Some([0.25, 0.75]),
            },
            10 => Command::AddMusic {
                media: 3,
                start: t(&mut rng),
                source_in: t(&mut rng),
                source_out: t(&mut rng),
            },
            _ => Command::SetMusicGain {
                id: music_id(&mut rng),
                gain_db: -6.5,
                fade_in: frames(5),
                fade_out: frames(5),
            },
        };
        let before = to_json(s.project());
        match s.execute(&cmd) {
            Ok(_) => applied += 1,
            Err(_) => assert_eq!(
                to_json(s.project()),
                before,
                "a refused command changes nothing"
            ),
        }
    }
    assert!(applied > 100);
    let fin = to_json(s.project());
    while s.undo().is_some() {}
    assert_eq!(to_json(s.project()), initial);
    while s.redo().is_some() {}
    assert_eq!(to_json(s.project()), fin);
}

#[test]
fn trims_from_one_drag_coalesce_into_one_undo() {
    let mut s = session();
    let id = insert(&mut s, 1, 0, 300);
    let before = s.project().clone();
    for f in [290, 280, 270, 260] {
        s.execute(&Command::TrimEnd {
            id,
            source_out: frames(f),
            drag: Some(7),
        })
        .unwrap();
    }
    s.execute(&Command::TrimEnd {
        id,
        source_out: frames(250),
        drag: Some(8),
    })
    .unwrap();
    assert_eq!(s.undo(), Some("trim_end"));
    assert_eq!(s.project().timeline.main[0].source_out, frames(260));
    s.undo();
    assert_eq!(**s.project(), *before);
    assert_eq!(s.undo_label(), Some("insert_clip"));
}

#[test]
fn layers_map_to_source_time() {
    let mut s = session();
    insert(&mut s, 1, 10, 40); // CFR, timeline 0..30
    insert(&mut s, 2, 0, 30); // VFR, timeline 30..60
    let title = s
        .execute(&Command::AddTitle {
            start: frames(0),
            duration: frames(15),
            text: "Hi".into(),
            style: None,
            position: None,
        })
        .unwrap()
        .unwrap();
    let music = s
        .execute(&Command::AddMusic {
            media: 3,
            start: frames(0),
            source_in: frames(0),
            source_out: frames(60),
        })
        .unwrap()
        .unwrap();
    s.execute(&Command::SetMusicGain {
        id: music,
        gain_db: -3.0,
        fade_in: frames(10),
        fade_out: Tick::ZERO,
    })
    .unwrap();
    let p = s.project();

    // Mid-frame time maps to the start of that timeline frame, then into the source.
    let layers = p.layers_at(frames(5) + Tick(1000));
    assert_eq!(
        layers[0],
        Layer::Video {
            clip: p.timeline.main[0].id,
            media: 1,
            source_time: frames(15)
        }
    );
    assert!(matches!(layers[1], Layer::Title(t) if t.id == title));
    assert_eq!(
        layers[2],
        Layer::Music {
            clip: music,
            media: 3,
            source_time: frames(5),
            gain_db: -3.0,
            fade: 0.5
        }
    );

    // VFR media gets the raw source time (the media layer picks the latest frame <= it).
    let layers = p.layers_at(frames(31));
    assert_eq!(
        layers[0],
        Layer::Video {
            clip: p.timeline.main[1].id,
            media: 2,
            source_time: frames(1)
        }
    );
    assert_eq!(layers.len(), 2, "title ended at frame 15");
    assert!(p.layers_at(frames(60)).is_empty());
    assert_eq!(p.duration(), frames(60));
}

#[test]
fn bad_input_is_refused() {
    let mut s = session();
    let id = insert(&mut s, 1, 0, 30);
    assert_eq!(
        s.execute(&Command::RippleDelete { id: 999 }),
        Err(Error::NotFound(999))
    );
    assert!(
        s.execute(&Command::InsertClip {
            media: 3,
            source_in: Tick::ZERO,
            source_out: secs(1),
            index: None
        })
        .is_err()
    );
    assert!(
        s.execute(&Command::InsertClip {
            media: 1,
            source_in: secs(70),
            source_out: secs(80),
            index: None
        })
        .is_err()
    );
    assert!(s.execute(&Command::Split { at: Tick::ZERO }).is_err());
    assert!(s.execute(&Command::MoveClip { id, index: 1 }).is_err());
    let m = s
        .execute(&Command::AddMusic {
            media: 3,
            start: Tick::ZERO,
            source_in: Tick::ZERO,
            source_out: secs(1),
        })
        .unwrap()
        .unwrap();
    assert!(
        s.execute(&Command::SetMusicGain {
            id: m,
            gain_db: f32::NAN,
            fade_in: Tick::ZERO,
            fade_out: Tick::ZERO
        })
        .is_err()
    );
    assert!(
        s.execute(&Command::SetMusicGain {
            id: m,
            gain_db: 0.0,
            fade_in: secs(2),
            fade_out: Tick(1)
        })
        .is_err()
    );
    let unknown = serde_json::from_str::<Command>(r#"{"cmd":"format_disk"}"#);
    assert!(unknown.is_err());
    let extra = serde_json::from_str::<Command>(r#"{"cmd":"join","id":1,"evil":true}"#);
    assert!(extra.is_err());
}

#[test]
fn commands_and_projects_round_trip_through_json() {
    let cmd = Command::TrimEnd {
        id: 4,
        source_out: frames(12),
        drag: Some(1),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.starts_with(r#"{"cmd":"trim_end""#));
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
    let minimal: Command =
        serde_json::from_str(r#"{"cmd":"edit_title","id":3,"text":"x"}"#).unwrap();
    assert_eq!(minimal.name(), "edit_title");

    let mut s = session();
    insert(&mut s, 1, 0, 90);
    s.execute(&Command::AddTitle {
        start: frames(3),
        duration: frames(30),
        text: "Ünïcode \"q\"".into(),
        style: None,
        position: Some([0.1, 0.9]),
    })
    .unwrap();
    let m = s
        .execute(&Command::AddMusic {
            media: 3,
            start: frames(2),
            source_in: frames(0),
            source_out: frames(40),
        })
        .unwrap()
        .unwrap();
    s.execute(&Command::SetMusicGain {
        id: m,
        gain_db: -12.3,
        fade_in: frames(4),
        fade_out: frames(6),
    })
    .unwrap();
    let json = to_json(s.project());
    assert!(json.contains("\"version\": 1"));
    let back = from_json(&json).unwrap();
    assert_eq!(back, **s.project());
    assert_eq!(to_json(&back), json);
}

#[test]
fn project_load_rejects_future_and_broken_files() {
    let json = to_json(&Project::new(Settings::default()));
    let future = json.replace("\"version\": 1", "\"version\": 2");
    assert_eq!(
        from_json(&future),
        Err(Error::UnsupportedVersion {
            found: 2,
            supported: FORMAT_VERSION
        })
    );
    assert!(
        from_json(&future)
            .unwrap_err()
            .to_string()
            .contains("version 2")
    );
    assert!(matches!(
        from_json(&json.replace("\"version\": 1,", "")),
        Err(Error::Json(_))
    ));
    assert!(matches!(from_json("not json"), Err(Error::Json(_))));
    // Ids already in the file are never handed out again.
    let mut s = session();
    let id = insert(&mut s, 1, 0, 30);
    let stale = to_json(s.project()).replace(&format!("\"next_id\": {}", id + 1), "\"next_id\": 1");
    assert!(from_json(&stale).unwrap().next_id > id);
}
