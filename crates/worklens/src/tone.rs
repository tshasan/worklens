use worklens_tui::Tone;

pub fn detect(configured: Option<&str>) -> Tone {
    if crate::text::no_color() {
        return Tone::NoColor;
    }
    let setting = std::env::var("WORKLENS_THEME").ok();
    match setting.as_deref().or(configured).map(str::trim) {
        Some("dark") => return Tone::Dark,
        Some("light") => return Tone::Light,
        Some("plain") => return Tone::Unknown,
        Some("nocolor") => return Tone::NoColor,
        _ => {}
    }
    query()
        .as_deref()
        .and_then(parse_osc11)
        .map(tone_of)
        .or_else(|| std::env::var("COLORFGBG").ok().as_deref().and_then(parse_colorfgbg))
        .unwrap_or(Tone::Unknown)
}

fn tone_of(rgb: [f64; 3]) -> Tone {
    let linear = |c: f64| {
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let l = 0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2]);
    if l < 0.179 { Tone::Dark } else { Tone::Light }
}

fn parse_osc11(reply: &[u8]) -> Option<[f64; 3]> {
    let text = String::from_utf8_lossy(reply);
    let start = text.find("]11;")? + 4;
    let body = &text[start..];
    let body = &body[..body.find(['\x07', '\x1b']).unwrap_or(body.len())];
    let channels = body.strip_prefix("rgb:").or_else(|| body.strip_prefix("rgba:"))?;
    let mut out = [0.0; 3];
    let mut parts = channels.split('/');
    for slot in &mut out {
        let hex = parts.next()?;
        if hex.is_empty() || hex.len() > 4 {
            return None;
        }
        let value = u32::from_str_radix(hex, 16).ok()?;
        let max = (1u32 << (4 * hex.len())) - 1;
        *slot = f64::from(value) / f64::from(max);
    }
    Some(out)
}

fn parse_colorfgbg(value: &str) -> Option<Tone> {
    let bg: u8 = value.rsplit(';').next()?.trim().parse().ok()?;
    Some(if bg == 7 || (9..=15).contains(&bg) {
        Tone::Light
    } else {
        Tone::Dark
    })
}

#[cfg(unix)]
fn query() -> Option<Vec<u8>> {
    use std::io::{IsTerminal as _, Write as _};
    use std::os::fd::AsFd as _;
    use std::time::{Duration, Instant};

    let stdin = std::io::stdin();
    if !stdin.is_terminal() || !std::io::stdout().is_terminal() {
        return None;
    }
    let mut out = std::io::stdout();
    out.write_all(b"\x1b]11;?\x1b\\\x1b[c").ok()?;
    out.flush().ok()?;

    let fd = stdin.as_fd();
    let deadline = Instant::now() + Duration::from_millis(200);
    let mut reply = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut fds = [rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN)];
        let timeout = rustix::event::Timespec::try_from(left).ok()?;
        if rustix::event::poll(&mut fds, Some(&timeout)).ok()? == 0 {
            break;
        }
        let mut buf = [0u8; 256];
        let n = rustix::io::read(fd, &mut buf).ok()?;
        if n == 0 {
            break;
        }
        reply.extend_from_slice(&buf[..n]);
        if has_da1_reply(&reply) {
            break;
        }
    }
    Some(reply)
}

#[cfg(not(unix))]
fn query() -> Option<Vec<u8>> {
    None
}

#[cfg_attr(not(unix), expect(dead_code, reason = "only the unix query reads replies"))]
fn has_da1_reply(bytes: &[u8]) -> bool {
    let Some(start) = bytes.windows(3).position(|w| w == b"\x1b[?") else {
        return false;
    };
    bytes[start + 3..]
        .iter()
        .find(|b| !(b.is_ascii_digit() || **b == b';'))
        .is_some_and(|b| *b == b'c')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_background_reply() {
        let rgb = parse_osc11(b"\x1b]11;rgb:1616/1616/1616\x1b\\\x1b[?62;c").unwrap();
        assert!((rgb[0] - 0x16 as f64 / 255.0).abs() < 0.001);
        assert_eq!(tone_of(rgb), Tone::Dark);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:ff/ff/ff\x07").map(tone_of), Some(Tone::Light));
        assert_eq!(parse_osc11(b"\x1b]11;rgb:fdfd/f6f6/e3e3\x07").map(tone_of), Some(Tone::Light));
        assert_eq!(parse_osc11(b"\x1b]11;rgba:0000/0000/0000/ffff\x07").map(tone_of), Some(Tone::Dark));
    }

    #[test]
    fn no_reply_or_garbage_is_unknown() {
        assert_eq!(parse_osc11(b"\x1b[?62;c"), None);
        assert_eq!(parse_osc11(b""), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:zz/00/00\x07"), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:00/00\x07"), None);
    }

    #[test]
    fn mid_grey_backgrounds_split_where_contrast_does() {
        assert_eq!(tone_of([0.45, 0.45, 0.45]), Tone::Dark);
        assert_eq!(tone_of([0.5, 0.5, 0.5]), Tone::Light);
    }

    #[test]
    fn colorfgbg_names_the_background_last() {
        assert_eq!(parse_colorfgbg("15;0"), Some(Tone::Dark));
        assert_eq!(parse_colorfgbg("0;15"), Some(Tone::Light));
        assert_eq!(parse_colorfgbg("0;default;7"), Some(Tone::Light));
        assert_eq!(parse_colorfgbg("nonsense"), None);
    }

    #[test]
    fn spots_the_end_of_the_da1_reply() {
        assert!(has_da1_reply(b"\x1b]11;rgb:0/0/0\x07\x1b[?62;22c"));
        assert!(!has_da1_reply(b"\x1b]11;rgb:0/0/0\x07\x1b[?62;2"));
        assert!(!has_da1_reply(b"\x1b]11;rgb:0/0/0\x07"));
    }
}
