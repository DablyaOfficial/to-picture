//! Разбор настоящих ответов yt-dlp -J (сохранены в tests/fixtures; обрезаны до нужных полей).
use serde_json::{Value, json};
use topicture_core::media::{AudioRule, audio_tracks, is_audio_only, official_subs, pick_audio, select_video, video_formats};
use topicture_core::preview::{playlist_data, playlist_entries, preview_data, site_of};
use topicture_core::PreviewResult;

fn load(name: &str) -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn video_fixture() {
    let info = load("video.json");
    let v = video_formats(&info);
    assert_eq!(v.len(), 12); // без звука и с высотой (18 — со звуком, 233/234 — только звук)
    let (f, copy) = select_video(&v, "best", "h264", true);
    assert_eq!((f.as_ref().unwrap().id.as_str(), copy), ("133", true));
    let (f, copy) = select_video(&v, "best", "h264", false);
    assert_eq!((f.unwrap().id.as_str(), copy), ("133", false));
    assert_eq!(select_video(&v, "best", "h265", true).0.unwrap().id, "133");
    assert_eq!(select_video(&v, "144", "h265", true).0.unwrap().id, "160");
    let t = audio_tracks(&info, false);
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].lang.as_str(), t[0].id.as_str()), ("und", "140"));
    assert_eq!(pick_audio(&t, &AudioRule::Rule("orig_ui".into()), "ru").len(), 1);
    assert_eq!(official_subs(&info), ["de", "en"]);
    assert!(!is_audio_only(&info));

    let d = preview_data(&info, "https://youtu.be/jNQXAC9IVRw", None);
    assert_eq!(d.title, "Me at the zoo");
    assert_eq!(d.channel, "jawed");
    assert_eq!(d.duration, Some(19.0));
    assert_eq!(d.height, Some(240));
    assert_eq!(d.vcodec.as_deref(), Some("avc1"));
    assert_eq!(d.heights, [144, 240]);
    assert_eq!(d.thumb_url.as_deref(), Some("https://i.ytimg.com/vi/jNQXAC9IVRw/maxresdefault.jpg"));
    assert!(d.thumb.is_none());
    assert_eq!(d.site, "youtube.com");
    assert!(d.youtube && d.auto_subs && !d.audio_only);
    assert_eq!(d.date.as_deref(), Some("20050424"));
    assert_eq!(d.subs, ["de", "en"]);
    assert_eq!(site_of(&json!({"webpage_url_domain": "m.vk.com"})), "vk.com");
    assert_eq!(site_of(&json!({"extractor_key": "TwitchClips"})), "TwitchClips");
}

#[test]
fn playlist_fixture() {
    let pl = load("playlist.json");
    let e = playlist_entries(&pl);
    assert_eq!(e.len(), 4);
    assert!(e.iter().all(|x| x.url.starts_with("https://www.youtube.com/watch?v=")));
    assert!(!e[0].title.is_empty());
    let PreviewResult::Playlist(p) = playlist_data(&pl, "https://www.youtube.com/playlist?list=PL590L5WQmH8coVKyRYxxQo8NG9O5NVLRW", "en", None) else { panic!() };
    assert_eq!(p.title, "Google I/O '26");
    assert_eq!(p.entries.len(), 4);
    let vid = e[0].url.split("v=").nth(1).unwrap();
    assert_eq!(p.thumb_url, Some(format!("https://i.ytimg.com/vi/{vid}/hqdefault.jpg")));
    assert!(p.duration.unwrap() > 0.0);

    let ch = load("channel_videos.json");
    let e = playlist_entries(&ch);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].url, "https://www.youtube.com/watch?v=jNQXAC9IVRw");
}

#[test]
fn channel_tabs_and_odd_entries() {
    // канал вкладками: берётся первая; голый id с ie_key Youtube дополняется до ссылки; прочее без http — пропуск
    let ch = json!({"_type": "playlist", "extractor": "youtube:tab", "title": "X", "entries": [
        {"_type": "playlist", "title": "X - Videos", "entries": [
            {"_type": "url", "ie_key": "Youtube", "url": "abc123", "title": "A", "duration": 10},
            null,
            {"_type": "url", "ie_key": "Other", "url": "zzz", "title": "skip"},
            {"_type": "url", "webpage_url": "https://vimeo.com/1", "title": "B"},
            {"_type": "url", "title": "no url"}
        ]},
        {"_type": "playlist", "title": "X - Shorts", "entries": [{"url": "https://www.youtube.com/shorts/q"}]}
    ]});
    let e = playlist_entries(&ch);
    let urls: Vec<_> = e.iter().map(|x| x.url.as_str()).collect();
    assert_eq!(urls, ["https://www.youtube.com/watch?v=abc123", "https://vimeo.com/1"]);
    assert_eq!(e[0].duration, Some(10.0));
    let PreviewResult::Error { error, raw } = playlist_data(&json!({"entries": []}), "u", "en", None) else { panic!() };
    assert_eq!(raw, "empty playlist");
    assert_eq!(error, "There are no videos in this playlist.");
}
