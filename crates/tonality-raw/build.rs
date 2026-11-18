//! Compacts the vendored lensfun database (`lensfun/*.xml`) into `$OUT_DIR/lensfun.json`:
//! only what lens correction uses, rectilinear lenses only.

use std::fmt::Write;
use std::path::Path;

fn num(n: roxmltree::Node, attr: &str, default: f64) -> f64 {
    n.attribute(attr)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.trim().chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {}
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn texts<'a>(n: roxmltree::Node<'a, 'a>, tag: &str) -> Vec<&'a str> {
    n.children()
        .filter(|c| c.has_tag_name(tag))
        .filter_map(|c| c.text())
        .map(str::trim)
        .collect()
}

fn list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
}

fn main() {
    let dir = Path::new("lensfun");
    println!("cargo:rerun-if-changed=lensfun");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "xml"))
        .collect();
    files.sort();

    let (mut mounts, mut cameras, mut lenses) = (Vec::new(), Vec::new(), Vec::new());
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let opts = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        let doc = roxmltree::Document::parse_with_options(&text, opts)
            .unwrap_or_else(|e| panic!("{f:?}: {e}"));
        for n in doc.root_element().children().filter(|n| n.is_element()) {
            match n.tag_name().name() {
                "mount" => {
                    let name = texts(n, "name")
                        .first()
                        .copied()
                        .unwrap_or_default()
                        .to_string();
                    mounts.push(format!(
                        "[{},{}]",
                        json_str(&name),
                        list(texts(n, "compat").iter().map(|c| json_str(c)))
                    ));
                }
                "camera" => {
                    let crop: f64 = texts(n, "cropfactor")
                        .first()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1.0);
                    cameras.push(format!(
                        "{{\"maker\":{},\"models\":{},\"mount\":{},\"crop\":{crop}}}",
                        json_str(texts(n, "maker").first().copied().unwrap_or_default()),
                        list(texts(n, "model").iter().map(|m| json_str(m))),
                        json_str(texts(n, "mount").first().copied().unwrap_or_default()),
                    ));
                }
                "lens" => {
                    if texts(n, "type")
                        .first()
                        .is_some_and(|t| *t != "rectilinear")
                    {
                        continue;
                    }
                    let crop: f64 = texts(n, "cropfactor")
                        .first()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1.0);
                    let aspect = texts(n, "aspect-ratio")
                        .first()
                        .and_then(|a| match a.split_once(':') {
                            Some((x, y)) => {
                                Some(x.trim().parse::<f64>().ok()? / y.trim().parse::<f64>().ok()?)
                            }
                            None => a.parse().ok(),
                        })
                        .unwrap_or(1.5);
                    let (mut dist, mut tca, mut vig) = (Vec::new(), Vec::new(), Vec::new());
                    for cal in n.children().filter(|c| c.has_tag_name("calibration")) {
                        for e in cal.children().filter(|c| c.is_element()) {
                            let model = e.attribute("model").unwrap_or_default();
                            let focal = num(e, "focal", 0.0);
                            match (e.tag_name().name(), model) {
                                ("distortion", "ptlens") => dist.push(format!(
                                    "[{focal},\"ptlens\",{},{},{}]",
                                    num(e, "a", 0.0),
                                    num(e, "b", 0.0),
                                    num(e, "c", 0.0)
                                )),
                                ("distortion", "poly3") => {
                                    dist.push(format!("[{focal},\"poly3\",{}]", num(e, "k1", 0.0)))
                                }
                                ("distortion", "poly5") => dist.push(format!(
                                    "[{focal},\"poly5\",{},{}]",
                                    num(e, "k1", 0.0),
                                    num(e, "k2", 0.0)
                                )),
                                ("tca", "linear") => tca.push(format!(
                                    "[{focal},\"linear\",{},{}]",
                                    num(e, "kr", 1.0),
                                    num(e, "kb", 1.0)
                                )),
                                ("tca", "poly3") => tca.push(format!(
                                    "[{focal},\"poly3\",{},{},{},{},{},{}]",
                                    num(e, "vr", 1.0),
                                    num(e, "vb", 1.0),
                                    num(e, "cr", 0.0),
                                    num(e, "cb", 0.0),
                                    num(e, "br", 0.0),
                                    num(e, "bb", 0.0)
                                )),
                                ("vignetting", "pa") => vig.push(format!(
                                    "[{focal},{},{},{},{},{}]",
                                    num(e, "aperture", 0.0),
                                    num(e, "distance", 1000.0),
                                    num(e, "k1", 0.0),
                                    num(e, "k2", 0.0),
                                    num(e, "k3", 0.0)
                                )),
                                _ => {}
                            }
                        }
                    }
                    if dist.is_empty() && tca.is_empty() && vig.is_empty() {
                        continue;
                    }
                    let mut s = String::new();
                    write!(
                        s,
                        "{{\"maker\":{},\"models\":{},\"mounts\":{},\"crop\":{crop},\"aspect\":{aspect},\"dist\":{},\"tca\":{},\"vig\":{}}}",
                        json_str(texts(n, "maker").first().copied().unwrap_or_default()),
                        list(texts(n, "model").iter().map(|m| json_str(m))),
                        list(texts(n, "mount").iter().map(|m| json_str(m))),
                        list(dist),
                        list(tca),
                        list(vig)
                    )
                    .unwrap();
                    lenses.push(s);
                }
                _ => {}
            }
        }
    }
    let out = format!(
        "{{\"mounts\":{},\"cameras\":{},\"lenses\":{}}}",
        list(mounts),
        list(cameras),
        list(lenses)
    );
    std::fs::write(
        Path::new(&std::env::var("OUT_DIR").unwrap()).join("lensfun.json"),
        out,
    )
    .unwrap();
}
