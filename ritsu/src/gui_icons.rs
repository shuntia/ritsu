// Helper to produce rotated loader SVG frames from a base lucide loader SVG (embedded via include_str!).
// This avoids depending on a specific lucide crate at build-time and allows smooth frame-based animation.

use std::fmt::Write;

pub fn make_spinner_frames() -> Vec<Vec<u8>> {
    const BASE: &str = include_str!("../assets/icons/loader.svg");
    let frames = 20usize; // number of discrete rotated frames (20 -> 1 rotation/sec at 50ms tick)
    let mut out = Vec::with_capacity(frames);

    for i in 0..frames {
        let angle = (i as f32) * (360.0 / frames as f32);
        // Insert a <g transform="rotate(angle 12 12)"> immediately after the opening <svg...> tag
        if let Some(open_end) = BASE.find('>') {
            let mut s = String::new();
            s.push_str(&BASE[..=open_end]);
            let _ = write!(&mut s, "<g transform=\"rotate({:.2} 12 12)\">", angle);
            s.push_str(&BASE[open_end + 1..]);
            if let Some(pos) = s.rfind("</svg>") {
                s.replace_range(pos..pos + 6, "</g></svg>");
            }
            out.push(s.into_bytes());
        } else {
            out.push(BASE.as_bytes().to_vec());
        }
    }

    out
}
