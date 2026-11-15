use std::{env, fs, path::Path};
fn main() {
    println!("cargo:rerun-if-env-changed=VISION_BUILD_COMMIT");
    println!(
        "cargo:rustc-env=VISION_BUILD_COMMIT={}",
        env::var("VISION_BUILD_COMMIT").unwrap_or_else(|_| "development".into())
    );
    let root = Path::new("../../web/dist");
    println!("cargo:rerun-if-changed=../../web/dist");
    let mut output = String::from("pub static ASSETS: &[(&str,&str,&[u8])] = &[\n");
    fn walk(root: &Path, dir: &Path, out: &mut String) {
        if !dir.exists() {
            return;
        }
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(root, &p, out);
                continue;
            }
            let mime = match p.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("json") => "application/json; charset=utf-8",
                Some("svg") => "image/svg+xml",
                Some("ttf") => "font/ttf",
                Some("txt") => "text/plain; charset=utf-8",
                _ => continue,
            };
            let route = format!(
                "/{}",
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            out.push_str(&format!(
                "({route:?},{mime:?},include_bytes!({:?})),\n",
                fs::canonicalize(&p).unwrap()
            ));
        }
    }
    walk(root, root, &mut output);
    output.push_str("];\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("web_assets.rs"),
        output,
    )
    .unwrap();
}
